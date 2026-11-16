# bulletin-probe

Minimal synthetic write+read probe for Bulletin Chain. On each iteration:

1. Submits a single-chunk `TransactionStorage.store` extrinsic with random bytes
   and waits for finality. Emits `probe.bulletin.store`.
2. Computes the Blake2b-256 content hash of the payload and queries
   `TransactionStorage.TransactionByContentHash` on chain to verify it is
   indexed. Emits `probe.bulletin.read`.

Designed as the SLI source for write-path and read-path SLOs. Atomic by
construction: 1 chunk, fixed bytes, sequential, fresh WS client per iteration.

## Setup

```bash
cd probe
npm install
npm run papi:generate        # writes .papi/descriptors against paseo-next-v2
cp .env.example .env         # fill in SENTRY_DSN and PROBE_MNEMONIC
npm start
```

`PROBE_MNEMONIC` must correspond to an account that's been authorised on the
target network. Generous allowance recommended so the probe runs for months
without refill governance.

## Env vars

| Name                       | Required | Default          | Notes                                  |
| -------------------------- | -------- | ---------------- | -------------------------------------- |
| `SENTRY_DSN`               | yes      | —                | Probe Sentry project DSN               |
| `PROBE_NETWORK`            | no       | `paseo-next-v2`  | Key from `src/networks.ts`             |
| `PROBE_MNEMONIC`           | no       | dev phrase       | Required on non-local networks         |
| `PROBE_INTERVAL_SEC`       | no       | `300`            | Sequential, never overlapping          |
| `PROBE_PAYLOAD_BYTES`      | no       | `65536`          | Must stay under single-chunk limit     |
| `PROBE_TX_TIMEOUT_SEC`     | no       | `180`            | Per-probe write deadline               |
| `PROBE_READ_TIMEOUT_SEC`   | no       | `10`             | Per-probe read deadline                |

## Span schema

Both spans share these attributes:

```
probe.network         e.g. paseo-next-v2
probe.tool_version    bulletin-probe@0.1.0
```

Write span (`probe.bulletin.store`) adds:

```
probe.payload_bytes   65536
probe.chunks          1
probe.tx_timeout      "true" when the deadline fires
probe.tx_dropped      "true" when the chain reports a dropped tx
```

Read span (`probe.bulletin.read`) adds:

```
probe.read_miss       "true" when TransactionByContentHash returned null
probe.read_timeout    "true" when the deadline fires
```

Seeded `"false"` so Sentry ratio queries (`count_if(probe.tx_timeout:true) / count()`)
work without `has:` gymnastics.

## Sentry SLOs

Define one SLO per signal. Latency targets are placeholders, dial after a few
days of real data.

**Write latency**

```
span.op:probe.bulletin.store probe.network:paseo-next-v2
good event: span.status:ok AND span.duration < 120000
target: 99% / 30d
```

**Read latency**

```
span.op:probe.bulletin.read probe.network:paseo-next-v2
good event: span.status:ok AND span.duration < 2000
target: 99.5% / 30d
```

## GitHub Actions cron

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

Each scheduled invocation fires once and exits (`timeout-minutes: 4`). For a
long-running deployment drop the timeout and run as a systemd unit or container
with `PROBE_INTERVAL_SEC=300`.

## Guardrails

1. `probe.chunks` must always be `1`. Multi-chunk probes pollute the SLI.
2. Iterations are strictly sequential. Don't swap the `while (!shuttingDown)`
   loop for `setInterval`; overlapping probes would contend on RPC.
