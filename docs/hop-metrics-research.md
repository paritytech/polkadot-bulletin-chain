# HOP Metrics (#609) - Research

Context gathered 2026-07-16 for [#609](https://github.com/paritytech/polkadot-bulletin-chain/issues/609)
"[HOP] Add some basic metrics". Sources: sc-hop at polkadot-sdk master, the pinned omni-node
commit (`.github/env`), the bulletin pallets, and the issue/PR graph below.

## 1. Where #609 sits

- **#635 (HOP design debt)**, Section A: observability is one of the four items that *must*
  resolve before Bulletin mainnet (Sep '26), explicitly "pool status / promotion success
  metrics + dashboards. Tracked: #609". Priority P1.
- **#622 (Observability & SLI stack)**: the umbrella. Covers PR #566 (synthetic store probe),
  PR #567 (event-to-Prometheus indexer), PR #533 (block-headroom observer). Notes it
  "relates to #609" and web3-storage#214.
- **#655 (audit scope)**: `sc-hop` (pool/RPC/promotion/rate-limiter) is Phase-2 audit scope,
  blocked on sdk#11988 (rate-limiter DoS) and sdk#12076 (pool metadata KV store).
- **Template**: sdk PR #12232 (merged) added bitswap metrics; #609 asks for the same
  treatment for HOP. sdk#12083 is the parent metrics design issue.
- **Adjacent**: #544 (HOP follow-ups), #626/PR #575 (submit signature v2, changes the submit
  payload), #639 (dedicated single-node HOP endpoints, prerequisite for black-box probing),
  #589 (dedup Rust HOP client), #627 (stress tooling), #580/#585 (testing frameworks).

## 2. Current observability: effectively zero

- **sc-hop has no metrics at all.** No `prometheus-endpoint` dependency, no Registry threaded
  anywhere. The only mention of "metrics" in the crate is a doc comment on
  `RateLimiter::tracked_senders`.
- **The only pool-level hook today is the `hop_poolStatus` RPC** returning
  `{entryCount, totalBytes, maxBytes}`. Per node, poll-only.
- **On-chain there is nothing HOP-specific.** `pallet-bulletin-hop-promotion` has zero events,
  zero storage items, zero errors. A promotion just calls `do_store`, so it surfaces as a
  generic `transaction-storage::Stored` event, indistinguishable from a user store unless the
  observer inspects the extrinsic (pallet/call index of `HopPromotion::promote`).
- **All `authorize_promote` rejections happen at tx-pool validation** (unauthorized signer,
  bad signature, oversized data, block at `MaxBlockTransactions`, timestamp skew > 48 h), so
  they are invisible on-chain: node-side only.
- **No HOP alerts** in `docs/oncall/bulletin.rules.yaml` or
  `docs/monitoring/bulletin-summit-alerts.yaml`; the SLO plan
  (`docs/metrics-monitoring-plan.md`) does not cover HOP.

## 3. Failure modes the metrics must catch

From the sc-hop source (constants in `substrate/client/hop/src/types.rs`): 24 h retention,
10 GiB pool, 256 MiB per-user cap, 5 min maintenance tick, promotion starts 2 h before expiry,
max 6 promotion attempts with exponential backoff, per-account rate limits
(60 submits/min, 128 MiB/min bandwidth).

These map directly to #635's "silent data-loss paths":

1. **Expired unpromoted.** `cleanup_expired` removes entries *regardless* of `meta.promoted`.
   The cleanup loop already has `meta.promoted` in hand, so a
   `removed_total{reason="expired_unpromoted"}` counter is nearly free. This counter is the
   data-loss signal; today only a freed-bytes total is logged.
2. **Promotion abandoned.** `get_promotable` filters out entries with
   `promotion_attempts >= MAX_PROMOTION_ATTEMPTS` (6); such entries are silently dropped from
   promotion and later expire. Detectable in `record_promotion_attempt`.
3. **Pool full.** There is no eviction; a full pool rejects inserts (`PoolFull`). This is the
   "collator-at-limit" policy gap the design review left open.
4. **Promotion starvation.** `promoter.promote()` returning Ok only means "accepted by the
   local tx pool"; inclusion is confirmed on the next tick via `is_promoted_on_chain`. If
   blocks stay full (promote runs at priority 0 in leftover blockspace, longevity 5), the
   submitted-but-unconfirmed backlog grows. Submitted and confirmed must be separate metrics.
5. **Promotion silently disabled.** `try_build_promoter` returning `None` degrades the
   maintenance task to cleanup-only. Needs a 0/1 gauge.
6. **Rate limiting and DoS.** Per-account buckets today; sdk#11988 (open) adds a global
   bucket because ~40 accounts can fill the 10 GiB pool in under two minutes at default rates.

## 4. Decentralization: three observability layers, not one

HOP is deliberately not a centralized service: each collator runs its own node-local pool, no
replication, clients pick nodes round-robin from a hard-coded list
(`console-ui/src/config/networks.ts` `hopNodes`; authority discovery is a #635 long-term item).
Consequences:

- **Node metrics only cover nodes we scrape.** Parity's Prometheus sees Parity-operated
  collators. External collators' pools are invisible to our Grafana. Instrumenting sc-hop
  still helps them (any operator gets the metrics from the standard `--prometheus-port`
  endpoint, same as every other substrate metric), but our dashboards must not claim
  network-wide coverage from them.
- **The chain is the only global, trustless truth.** Promote-originated `Stored` events are
  visible to everyone. The indexer (PR #567) can derive `bulletin_hop_promotions_total` per
  network today, with no SDK change, by matching the extrinsic's pallet/call index. This is
  the only HOP signal that covers *all* collators, including ones nobody scrapes.
- **The user experience is a third thing.** A user's round trip depends on which node the
  round-robin picked, and sender/receiver must hit the same node (the multi-replica public
  RPC breaks round trips, hence PR #581 and #639's dedicated `*-hop-0` endpoints). Only a
  synthetic black-box probe (a scheduled `hop_round_trip`-style job) measures this.
- **Aggregation semantics.** Pool gauges sum across nodes into "total pool bytes", but the
  SLO is per-node: one full pool loses that node's users' data even if the fleet average is
  fine. Alert per instance, not on the sum.
- **Cardinality and privacy.** No per-account labels on a public-ish metrics endpoint:
  unbounded cardinality and it leaks who is submitting. Rate-limit rejections get a `reason`
  label only.

So the plan is three complementary layers:

| Layer | Covers | Mechanism |
|---|---|---|
| Node (sc-hop Prometheus) | pool health, RPC outcomes, promotion funnel, data loss, per scraped collator | SDK change (this issue's core) |
| Chain (indexer, PR #567) | promotions network-wide, all collators | extrinsic inspection, no SDK change |
| Probe (synthetic round trip) | end-to-end user experience incl. submit-to-promotion latency | needs #639 endpoints |

This matches web3-storage#214's direction: off-chain components expose metrics via the same
`substrate-prometheus-endpoint` mechanism as SDK nodes.

## 5. Proposed node-side metric set

Mirror `bitswap_metrics.rs` (sdk PR #12232): private `Inner` registered against
`Option<&Registry>`, public wrapper whose recorders are no-ops without a registry, label
values as consts. Prefix `substrate_hop_*`. The pool `Arc` is shared by RPC and the
maintenance task, so metrics living in `HopDataPool` are visible to all components.
Implemented in [sdk PR #12662](https://github.com/paritytech/polkadot-sdk/pull/12662).

Pool (`pool.rs`):

| Metric | Type | Labels |
|---|---|---|
| `substrate_hop_pool_entries` / `substrate_hop_pool_bytes` / `substrate_hop_pool_max_bytes` | gauges | snapshot-set from the authoritative pool counters on every mutation (never inc/dec'd; a `Gauge<U64>` wraps on underflow), same pattern as the fork-aware txpool and statement store |
| `substrate_hop_pool_inserts_total` | counter | `outcome` = `ok`, `no_recipients`, `duplicate_recipient`, `empty_data`, `rate_limited`, `pool_full`, `user_quota_exceeded`, `duplicate_entry`, `io_error` |
| `substrate_hop_pool_removed_total` | counter | `reason` = `acked`, `expired_promoted`, **`expired_unpromoted`**, `corrupt` |
| `substrate_hop_pool_inserted_bytes_total` | counter | - |

RPC (`rpc.rs`, single error funnel via `HopError -> ErrorObjectOwned`):

| Metric | Type | Labels |
|---|---|---|
| `substrate_hop_rpc_requests_total` | counter | `method` = `submit`/`claim`/`ack`/`pool_status`, `outcome` = `ok` or HopError variant (incl. `not_found`, `already_claimed`, `not_authorized`, `invalid_signature`, `data_too_large`, ...) |

No per-method duration histogram: the node's RPC middleware already exposes
`substrate_rpc_calls_started/finished/time{method}` (with `is_error`) for every RPC method,
including `hop_*`. The counter above only adds error-variant granularity the middleware
cannot see.

Promotion (`promotion.rs`):

| Metric | Type | Labels |
|---|---|---|
| `substrate_hop_promotion_submissions_total` | counter | `outcome` = `submitted` (accepted by local tx pool), `failed` |
| `substrate_hop_promotions_confirmed_total` | counter | - (from `mark_promoted`, i.e. actually on-chain) |
| `substrate_hop_promotions_abandoned_total` | counter | - (attempt cap reached) |
| `substrate_hop_promotion_backlog` | gauge | - (full in-window unpromoted backlog, refreshed per tick) |
| `substrate_hop_promotion_enabled` | gauge 0/1 | - |
| `substrate_hop_maintenance_tick_duration_seconds` | histogram | - |

Rate limiter: rejections surface as `inserts_total{outcome="rate_limited"}`. A per-bucket
`reason` split (`requests`/`bandwidth`/`global`) is deferred until sdk#11988 settles the
limiter shape.

Plumbing: add `prometheus-endpoint` to sc-hop's Cargo.toml; pass `Option<&Registry>` into
`HopParams::build_pool` and `build_maintenance_task`. In
`cumulus/polkadot-omni-node/lib/src/common/spec.rs` the `prometheus_registry` is already in
scope right above the HOP wiring, so node integration is a two-line change.

## 6. Dashboards and alerts

Grafana (per chain, per instance):

- Pool headroom: `pool_bytes / pool_max_bytes` per node, plus entry count.
- Insert outcomes stacked rate; `pool_full` and `rate_limited` called out.
- Promotion funnel: backlog -> attempts(submitted/failed) -> confirmed; abandoned as a stat.
- Data-loss panel: `expired_unpromoted` + `abandoned` (should be flat zero).
- RPC: outcome rates from `substrate_hop_rpc_requests_total`; p95/p99 durations from the
  existing middleware metric `substrate_rpc_calls_time{method=~"hop_.*"}`; `claim` `not_found` ratio
  (receiver hitting the wrong node is the node-local-pool symptom).

Alert candidates for `bulletin.rules.yaml`:

| Alert | Expr sketch | Severity |
|---|---|---|
| HOP data loss | `increase(substrate_hop_pool_removed_total{reason="expired_unpromoted"}[1h]) > 0` or any `abandoned` | critical |
| HOP pool near cap | `pool_bytes / pool_max_bytes > 0.8` for 30m (per instance) | warning; > 0.95 critical |
| HOP promotion failing | `failed/(submitted+failed) > 0.5` over 1h, or backlog growing while confirmed flat | warning |
| HOP promotion disabled | `substrate_hop_promotion_enabled == 0` on a HOP-enabled node | critical |

Chain layer: add a `bulletin_hop_promotions_total` counter to the indexer (PR #567) by
matching `HopPromotion::promote` extrinsics in finalized blocks. Probe layer: schedule a
`hop_round_trip` variant recording submit-to-claim and submit-to-promotion latency
histograms; blocked on #639 for westend/previewnet.

## 7. Sequencing and interactions

1. **sdk#12076 (pool metadata -> parity-db KV) is open and rewrites pool internals.** The
   insert/expiry/RPC hook sites survive, but landing metrics before or after it decides who
   rebases. Coordinate; the metrics PR is probably easier to rebase than #12076.
2. **sdk#11988** adds the `global` rate-limit reason; design the label set to accommodate it.
3. **PR #575 (submit sig v2)** changes the submit payload, not the metric sites, but any
   probe/stress client reconstructing the payload tracks it.
4. **#654 (stable2606 bump)**: the metrics PR lands on sdk master and must be included in the
   omni-node release the repo pins (`.github/env` `POLKADOT_NODE_VERSION`). The audit (#655)
   runs against stable2606, and Phase-2 sc-hop audit wants #11988/#12076 closed; metrics
   riding the same bump keeps one release train.
5. **Deadline**: #635 Section A makes this P1 for Bulletin mainnet (Sep '26).

## 8. Open questions

- **On-chain promotion event?** Today promotions are only distinguishable by extrinsic
  inspection. A pallet event would make the chain layer trivial, but it is a runtime change
  during audit-freeze; the indexer approach needs no runtime change. Recommend indexer now,
  revisit an event with the #635 long-term redesign.
- **Extend `hop_poolStatus`?** Exposing promotion backlog / attempt stats via the existing
  RPC would give external collator operators visibility without Prometheus, at the cost of a
  public API surface. Optional follow-up.
- **Per-user usage gauge**: skipped deliberately (cardinality, privacy). The per-user quota
  rejection counter covers the operational question.
