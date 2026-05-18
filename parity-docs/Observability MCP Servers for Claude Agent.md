# Observability MCP Servers for Claude Agent

**Author(s):** Crosschain Team
**Status:** Draft
**Created:** 2026-03-06
**Last Updated:** 2026-03-06

---

### 1. Overview

This document specifies which MCP servers to deploy so the Claude SRE agent (see [Agentic SRE](Agentic%20SRE%20-%20Claude%20as%20First-Line%20On-Call.md)) has the same observability access as a human engineer with Grafana, kubectl, and polkadot.js open.

### 2. What the Bulletin Chain Exposes Today

| Data Source | How It's Produced | Where It Lives |
|---|---|---|
| **Prometheus metrics** | Polkadot SDK auto-exposes on `:9615` — block height, finality, peers, tx pool, BABE, GRANDPA, network stats | Scraped by Prometheus (once Phase 0 monitoring is deployed) |
| **Structured logs** | `tracing` crate with per-component targets: `runtime::transaction-storage`, `runtime::validator-set`, `runtime::relayer-set`, `sub-libp2p::bitswap`, `litep2p::ipfs::bitswap` | stdout → Loki (once deployed) |
| **Runtime events** | Pallet events (`ProofChecked`, `Stored`, validator/relayer set changes) | On-chain, queryable via RPC |
| **Telemetry** | Optional structured events to telemetry endpoints | Polkadot telemetry service |

Key metrics for alerting (from the SRE doc):

```promql
# Finality stall
(substrate_block_height{status="best"} - substrate_block_height{status="finalized"}) > 20

# Network isolation
substrate_sub_libp2p_peers_count < 3

# Tx pool backlog
substrate_ready_transactions_number > 100
```

### 3. MCP Server Stack

Three servers give the agent full observability. A fourth adds chain-native access.

#### 3.1 Grafana — Metrics, Logs, Dashboards, Alerts

The official [mcp-grafana](https://github.com/grafana/mcp-grafana) server covers Prometheus, Loki, dashboards, alerting, incidents, and OnCall through a single connection.

**Key tools the agent uses:**

| Category | Tools | Agent Use Case |
|---|---|---|
| Prometheus | `query_prometheus`, `list_prometheus_metric_names`, `list_prometheus_label_values` | Check finality gap, peer count, block rate |
| Loki | `query_loki_logs`, `query_loki_patterns` | Search node logs for errors, correlate with alerts |
| Dashboards | `search_dashboards`, `get_dashboard_by_uid`, `get_panel_image` | Inspect existing dashboards, render panels for issue comments |
| Alerting | `alerting_manage_rules` | Read alert rule definitions to understand what fired |
| Incidents | `list_incidents`, `create_incident`, `add_activity_to_incident` | Track and document resolution |
| Sift | `find_error_patterns`, `find_slow_requests` | AI-powered log analysis |

**Configuration:**

```json
{
  "mcpServers": {
    "grafana": {
      "command": "uvx",
      "args": ["mcp-grafana", "--disable-write"],
      "env": {
        "GRAFANA_URL": "https://grafana.bulletin.internal",
        "GRAFANA_SERVICE_ACCOUNT_TOKEN": "${GRAFANA_TOKEN}"
      }
    }
  }
}
```

Use `--disable-write` in production. The agent only needs read access for diagnosis. Drop the flag for incident management (create/update incidents).

**Requirements:** Grafana 9.0+, service account with `datasources:query` + `dashboards:read` RBAC.

#### 3.2 Kubernetes — Pod Logs, Restarts, Cluster State

The official [kubernetes-mcp-server](https://github.com/containers/kubernetes-mcp-server) provides kubectl-equivalent access. Required if nodes run on k8s.

**Key tools:**

| Tool | Agent Use Case |
|---|---|
| `pods_list`, `pods_get` | Find collator/validator pods, check status |
| `pods_log` | Tail container logs when Loki is unavailable or for real-time debugging |
| `pods_exec` | Run diagnostic commands inside a node container |
| `pods_top`, `nodes_top` | Check CPU/memory pressure |
| `events_list` | Detect OOMKills, CrashLoopBackOff, scheduling failures |

**Configuration:**

```json
{
  "mcpServers": {
    "kubernetes": {
      "command": "npx",
      "args": ["-y", "kubernetes-mcp-server@latest", "--read-only"]
    }
  }
}
```

Use `--read-only` until Phase 2. The `/restart-collator` skill (Phase 2) will need write access with `--disable-destructive` to allow pod restarts but prevent deletions.

**Requirements:** Valid kubeconfig with access to the bulletin chain namespace(s).

#### 3.3 Chain RPC — On-Chain State

A custom MCP server wrapping Substrate's JSON-RPC. This is the one server we need to build.

**Tools to implement:**

| Tool | RPC Method | Agent Use Case |
|---|---|---|
| `get_finalized_head` | `chain_getFinalizedHead` | Confirm finality stall |
| `get_block` | `chain_getBlock` | Inspect block contents |
| `query_storage` | `state_getStorage` | Read pallet storage (validator set, relayer set, retention period) |
| `get_runtime_version` | `state_getRuntimeVersion` | Verify runtime version after upgrade |
| `get_health` | `system_health` | Node sync status + peer count |
| `pending_extrinsics` | `author_pendingExtrinsics` | Check tx pool state |

**Configuration:**

```json
{
  "mcpServers": {
    "chain-rpc": {
      "command": "node",
      "args": [".claude/mcp-servers/chain-rpc/index.js"],
      "env": {
        "RPC_ENDPOINTS": "wss://bulletin-westend.parity.io,wss://bulletin-polkadot.parity.io"
      }
    }
  }
}
```

Read-only by design — no signing keys, no extrinsic submission. Extrinsic submission is handled by Skills with explicit user approval.

#### 3.4 Optional: Standalone Servers

If Grafana is not yet deployed (Phase 0 in progress), use standalone servers as a bridge:

| Need | Server | Repo |
|---|---|---|
| Metrics only | prometheus-mcp-server | [pab1it0/prometheus-mcp-server](https://github.com/pab1it0/prometheus-mcp-server) |
| Logs only | loki-mcp | [grafana/loki-mcp](https://github.com/grafana/loki-mcp) |
| Traces | opentelemetry-mcp | [traceloop/opentelemetry-mcp-server](https://github.com/traceloop/opentelemetry-mcp-server) |
| Errors | sentry-mcp | [getsentry/sentry-mcp](https://github.com/getsentry/sentry-mcp) |

Replace these with mcp-grafana once Grafana is live.

### 4. How It Fits the SRE Agent Flow

```
Alert fires (Alertmanager → GitHub issue)
         │
         ▼
┌─ Claude Agent ─────────────────────────────────────┐
│                                                     │
│  1. Read alert context from issue body              │
│                                                     │
│  2. Diagnose ──┬── Grafana MCP ── PromQL query      │
│                ├── Grafana MCP ── Loki log search    │
│                ├── Chain RPC MCP ── finalized head    │
│                └── K8s MCP ── pod status, events     │
│                                                     │
│  3. Correlate: alert + metrics + logs + chain state  │
│                                                     │
│  4. Resolve ──┬── /release (runtime upgrade)         │
│               ├── /rotate-keys (session keys)        │
│               └── /restart-collator (k8s restart)    │
│                                                     │
│  5. Verify ───┬── Chain RPC ── finality resumed?     │
│               ├── Grafana MCP ── alert cleared?      │
│               └── K8s MCP ── pod healthy?            │
│                                                     │
│  6. Close issue with diagnosis + resolution summary  │
└─────────────────────────────────────────────────────┘
```

### 5. Security

| Concern | Mitigation |
|---|---|
| Grafana token leak | Store in CI secrets / env, never in repo. Service account with minimal RBAC. |
| K8s write access | `--read-only` by default. Write access only for `/restart-collator` with human approval gate. |
| Chain RPC abuse | Read-only server, no signing keys. No `author_submitExtrinsic`. |
| Sensitive log data | Loki queries scoped to bulletin chain namespace. No PII in substrate logs. |
| MCP server compromise | Pin versions, run in sandboxed containers, audit dependencies. |

### 6. Implementation Plan

| Phase | When | What | Depends On |
|---|---|---|---|
| **0a** | Mar 2026 | Deploy Prometheus + Loki + Grafana for Bulletin Chain | Infra team |
| **0b** | Mar 2026 | Add `mcp-grafana` to Claude agent config (read-only) | 0a |
| **0c** | Mar 2026 | Build Chain RPC MCP server (read-only, ~2 eng-days) | — |
| **1** | Apr 2026 | Add `kubernetes-mcp-server` (read-only) | K8s access provisioned |
| **2** | May 2026 | Enable K8s write for `/restart-collator`, integrate with approval gates | SRE Phase 2 |

### 7. Alternatives Considered

| Approach | Why Not |
|---|---|
| **Bash + curl to APIs** | Fragile, no schema, no discoverability. MCP gives the agent typed tools it can reason about. |
| **Build all MCP servers custom** | mcp-grafana and kubernetes-mcp-server already exist and are maintained by Grafana/Red Hat. Only Chain RPC needs custom work. |
| **Give agent direct Grafana UI access** | Requires browser automation, slow, brittle. MCP provides structured data the agent can reason over directly. |
| **Skip Grafana, query Prometheus/Loki directly** | Loses dashboards, alerting, incidents, OnCall, and Sift. Grafana MCP is a superset. |
