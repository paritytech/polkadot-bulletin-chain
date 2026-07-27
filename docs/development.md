# Local Development

Building, fetching binaries, and running local networks. For the shortest path to a running node, see the [Quickstart](../README.md#quickstart) in the README.

## One-time setup

Install [`just`](https://github.com/casey/just) — every recipe in this repo is wired through it, and the CI workflows call the same recipes:

```bash
cargo install just --locked
just --list   # see all recipes
```

## polkadot-sdk binaries

All external binaries (`polkadot`, `polkadot-omni-node`, `polkadot-prepare-worker`, `polkadot-execute-worker`, `chain-spec-builder`, `frame-omni-bencher`, `try-runtime`, `zombienet`) are fetched on demand by `scripts/get_polkadot_binaries.sh` and cached under `./.polkadot-binaries/` (gitignored, repo-local — no `$HOME` writes).

Five env vars in `.github/env` pin the version of each group:

| Variable | Drives |
|---|---|
| `POLKADOT_NODE_VERSION` | `polkadot`, 2 workers, `polkadot-omni-node` |
| `FRAME_OMNI_BENCHER_VERSION` | `frame-omni-bencher` |
| `CHAIN_SPEC_BUILDER_VERSION` | `chain-spec-builder` |
| `TRY_RUNTIME_VERSION` | `try-runtime` |
| `ZOMBIENET_VERSION` | `zombienet` (release-tag only) |

Each value is either:
- **a release tag** (e.g. `polkadot-stable2603`) — script downloads the prebuilt asset for your platform (Linux x86_64 or macOS arm64) and verifies its `.sha256` companion file, OR
- **a 40-char commit hash** — script clones `polkadot-sdk` / `try-runtime-cli` once into `./.polkadot-binaries/_src/`, checks out the commit, and builds with `SKIP_WASM_BUILD=1`.

Override at the shell to pin a different version for one session:

```bash
POLKADOT_NODE_VERSION=polkadot-stable2603 just binaries-polkadot
POLKADOT_NODE_VERSION=d6a4f5977b39bf5e5152e2f2bb6719ea92b992ea just binaries-polkadot
```

Useful recipes:

```bash
just binaries-polkadot              # fetch / build polkadot + workers + omni-node
just binaries-chain-spec-builder    # fetch / build chain-spec-builder
just binaries-bencher               # frame-omni-bencher
just binaries-try-runtime           # try-runtime CLI
just binaries-zombienet             # zombienet (release-only)
just binaries-all                   # cold-cache convenience: every group

just chain-spec westend             # build runtime + emit zombienet/bulletin-westend-spec.json
just chain-spec paseo               # same for paseo

just test-pallets                   # pallet unit tests
just test-zombienet-auto-renew      # auto-renew e2e suite (matrix: westend|paseo)
just test-zombienet-sync            # sync e2e suite
just bench <args>                   # frame-omni-bencher with extra args
just try-runtime <args>             # try-runtime CLI with extra args
```

## Zombienet

Local parachain networks can be spun up using the configurations in `zombienet/`:

- `bulletin-westend-local.toml` - Local Westend relay + Bulletin parachain
- `bulletin-paseo-local.toml` - Local Paseo relay + Bulletin parachain

## Examples

The `examples/` directory contains JavaScript, TypeScript, and Rust scripts demonstrating chain interaction:

- Authorization and storage workflows (WebSocket RPC and Smoldot light client)
- Content-addressed (preimage) authorization
- Chunked data storage with DAG-PB manifests
- Large file handling with parallel uploads
- Auto-renewal monitoring
- Runtime upgrades

See [examples/README.md](../examples/README.md) for setup and usage.
