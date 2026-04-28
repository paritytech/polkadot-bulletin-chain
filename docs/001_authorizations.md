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

At sustained-peak block usage, the current window's fresh `store` data alone is ~1.73 TiB. Whatever is renewed from the previous window sits on top of that, then renewals from the window before, and so on. After N windows of full usage where everything keeps getting renewed, disk requirement is roughly **N × 1.73 TiB**. ⇒ motivates a **hard limit** on permanent storage allowance (per account and chain-wide).

## Storage Types

Conceptually there will be 2 types of storage:

- **Temporary storage** — happens through the `store` call.
- **Permanent storage** — happens through the `renew` call (this can also be initiated through the auto-renewal flow).

## Allowance Limits

There are 2 limits on allowances:

- A **soft limit** on temporary storage allowance.
- A **hard limit** on permanent storage allowance.

Once the soft limit is crossed, the `store` calls post this for that account will be on lower priority — meant to utilise the block space when available.

### Allowance storage and `authorize_account`

- One `AuthorizationExtent` per account is kept in the `Authorizations` storage map, keyed by `AuthorizationScope::Account(AccountId)`.
- `AuthorizationExtent` carries `{ bytes, bytes_permanent, bytes_allowance }`: store/renew usage counters and the cap.
- `authorize_account(who, bytes)` **sets** `bytes_allowance = bytes` on an unexpired entry — it does **not** add to the existing cap. Used (`bytes` / `bytes_permanent`) counters are preserved, so a re-authorize can lower the effective remaining capacity but never grants extra by accident. Expiration is not pushed back; use `refresh_account_authorization` for that.
- If the entry is missing or expired, `authorize_account` creates a fresh one with `bytes = 0`, `bytes_permanent = 0`.

### Refresh authorization

`refresh_account_authorization(who)` is how a soft-limit-exhausted account gets back below the limit:

- Resets `bytes` to `0` — the user is in-budget again and regains the full priority boost on subsequent `store` calls.
- Does **not** reset `bytes_permanent` — renewed data stays on chain across refresh cycles, so the permanent-storage accounting survives. Resetting it would let a holder commit unbounded permanent storage by repeatedly refreshing.
- Extends `expiration` by another `AuthorizationPeriod`. `bytes_allowance` is unchanged.
- Origin: only `T::Authorizer` (e.g. PoP) can call it. Users cannot self-refresh; PoP controls the cadence at which a user's soft limit clears.

### Soft-limit implementation ([PR #448](https://github.com/paritytech/polkadot-bulletin-chain/pull/448))

- `check_authorization` does **not** reject `store` calls over the soft limit; it saturates `extent.bytes` upward and lets the tx validate.
- The `AllowanceBasedPriority` transaction extension adds a priority boost via a runtime-selected `BoostStrategy`.
- `FlatBoost`: `ALLOWANCE_PRIORITY_BOOST` while `bytes < bytes_allowance`, `0` once over.
- Net effect: in-budget `store` txs sort strictly above over-budget ones; over-budget txs ride leftover block space (no rejection, just demotion). Pool nonce/arrival ordering breaks ties among in-budget signers.

#### Example

PoP authorizes Alice for 64 MiB (`bytes_allowance = 64 MiB`, `bytes = 0`).

| Step | Call | After | Boost |
|---|---|---|---|
| 1 | `store(30 MiB)` | `bytes = 30 MiB` | `ALLOWANCE_PRIORITY_BOOST` (in-budget) |
| 2 | `store(30 MiB)` | `bytes = 60 MiB` | `ALLOWANCE_PRIORITY_BOOST` (still in-budget) |
| 3 | `store(10 MiB)` | `bytes = 70 MiB` (saturates over 64 MiB) | `0` (over soft limit) |
| 4 | `store(1 MiB)` | `bytes = 71 MiB` | `0` (still over) |
| 5 | PoP calls `refresh_account_authorization(Alice)` | `bytes = 0`, `bytes_allowance = 64 MiB` (unchanged), expiration extended | — |
| 6 | `store(20 MiB)` | `bytes = 20 MiB` | `ALLOWANCE_PRIORITY_BOOST` (in-budget again) |

Steps 1–2 ride normal high priority. From step 3 onward Alice's `store` calls still validate and consume `bytes`, but the `AllowanceBasedPriority` extension contributes `0`, so they queue behind every in-budget signer and only land in blocks with leftover space. Step 5 (refresh by PoP) clears `bytes` and Alice is back in-budget for step 6.

## Auto Renewal

There are some details with Auto Renewal that need to be closed — Cisco had some ideas. Karol Kokoszka — FYI.

## Capacity Planning

Track the overall space utilisation for permanent storage and act when it's close to full — either through:

- A referendum to increase the disk space of collators, or
- Spawning another bulletin chain.


## TODO

- summarize number for PoP (64 MiB) and PoP-lite (2 MiB) and how many user we can have when 1.7 TiB for permanent storage.
- summarize all the impl details

## TODO impl

- track when renewed content is expired and return allowance back to the account and decrease bytes_permanent
- check accounting allowances