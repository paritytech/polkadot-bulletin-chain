# bulletin-probe

Atomic synthetic probe for Bulletin Chain. Submits a single, fixed-size
`TransactionStorage.store` extrinsic on a fixed cadence and emits Sentry spans
matching `bulletin-deploy`'s schema, tagged with `deploy.probe`.

This is the **SLI source** for the write-path SLO. Sentry's existing aggregations
(p90 / p95 / count by `span.op`) work without any extra dashboards — just filter
on `deploy.probe:slo-*` to isolate probe samples from real users.

## How it differs from real-user spans

| Property                 | Real-user `deploy.chunk-upload` | Probe `deploy.chunk-upload` |
| ------------------------ | ------------------------------- | --------------------------- |
| Chunks per run           | 1 to thousands (parallel)       | exactly 1                   |
| Payload size             | varies                          | fixed (default 64 KB)       |
| Sibling contention       | yes                             | none                        |
| Cadence                  | when users deploy               | fixed (default 5 min)       |
| Tag `deploy.probe`       | unset                           | `slo-<network>`             |

The probe is what you write an SLO against. The real-user widget remains useful
as a workload-sensitivity view.

## Setup

```bash
cd probe
npm install
npm run papi:generate        # writes .papi/descriptors against paseo-next-v2
cp .env.example .env         # then fill in SENTRY_DSN and PROBE_MNEMONIC
npm start
```

`PROBE_MNEMONIC` must correspond to an account that's been authorised on the
target network (`authorize_account` with at least a few hundred bytes of
allowance, enough for one extrinsic per probe interval).

## Env vars

| Name                     | Required | Default              | Notes                                     |
| ------------------------ | -------- | -------------------- | ----------------------------------------- |
| `SENTRY_DSN`             | yes      | —                    | Use the same project as `bulletin-deploy` |
| `PROBE_NETWORK`          | no       | `paseo-next-v2`      | Key from `src/networks.ts`                |
| `PROBE_MNEMONIC`         | no       | dev phrase (`//Alice`-equivalent root) | Required for non-local networks |
| `PROBE_INTERVAL_SEC`     | no       | `300`                | Sequential, never overlapping             |
| `PROBE_PAYLOAD_BYTES`    | no       | `65536`              | Must stay under the single-chunk limit    |
| `PROBE_TX_TIMEOUT_SEC`   | no       | `180`                | Hard deadline per probe                   |

## Sentry SLO

Define the SLO in Sentry once, against this query:

```
span.op:deploy.chunk-upload deploy.probe:slo-*
```

- **Good event**: `span.status:ok AND span.duration < 2min`
- **Target**: 99% over 30 days
- **Burn-rate alerts**: Sentry defaults (fast + slow) are fine

To compare real-user vs probe in the existing Phase Breakdown widget, clone it
twice with these filters:

```
# user-experienced (real deploys, contended)
span.op:deploy.chunk-upload !deploy.probe:slo-*

# isolated (probe baseline, atomic)
span.op:deploy.chunk-upload deploy.probe:slo-* deploy.chunks.total:1
```

## Cron deploy (GitHub Actions)

```yaml
name: bulletin-probe
on:
  schedule:
    - cron: "*/5 * * * *"
  workflow_dispatch:
jobs:
  probe:
    runs-on: ubuntu-latest
    timeout-minutes: 4
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: "22" }
      - run: npm ci --prefix probe
      - run: npm run papi:generate --prefix probe
      - run: npm start --prefix probe
        env:
          SENTRY_DSN:        ${{ secrets.BULLETIN_PROBE_DSN }}
          PROBE_NETWORK:     paseo-next-v2
          PROBE_MNEMONIC:    ${{ secrets.BULLETIN_PROBE_MNEMONIC }}
          PROBE_INTERVAL_SEC: "60"
```

In this mode `PROBE_INTERVAL_SEC` is shorter than the cron cadence because the
workflow is killed after one run by `timeout-minutes`. Each scheduled invocation
fires once and exits.

For long-running deployment (one process, multiple iterations) drop the
`timeout-minutes` cap and host as a systemd unit or container with
`PROBE_INTERVAL_SEC=300`.

## Guardrails

The probe is designed to be **atomic** and **non-contending**. Two invariants
worth keeping if you change the code:

1. `deploy.chunks.total` must always be `1`. If the payload ever needs chunking,
   fail loudly. Multi-chunk probes pollute the isolated SLI population.
2. Probe iterations are strictly sequential. Don't replace the `for (;;)` loop
   with `setInterval` — overlapping probes would contend with their own RPC
   submissions and corrupt the baseline.
