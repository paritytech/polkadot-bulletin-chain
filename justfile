# Same recipes run in CI and locally. Binaries are resolved by
# `scripts/get_polkadot_binaries.sh` and cached under `./.polkadot-binaries/`.
# Versions come from `.github/env`; override per-shell with e.g.
# `POLKADOT_NODE_VERSION=<tag-or-40-hex-commit> just …`.
# Recognised version vars: POLKADOT_NODE_VERSION, FRAME_OMNI_BENCHER_VERSION,
# CHAIN_SPEC_BUILDER_VERSION, TRY_RUNTIME_VERSION, ZOMBIENET_VERSION (release-tag only).

set shell := ["bash", "-eu", "-o", "pipefail", "-c"]
set positional-arguments

# Shell-exported vars still override file values.
set dotenv-load := true
set dotenv-filename := ".github/env"

# Default: list recipes.
default:
    @just --list

# ---------------------------------------------------------------------------
# Binary fetchers — each prints the directory holding the requested binaries.
# ---------------------------------------------------------------------------

# Fetch / build polkadot + 2 workers + polkadot-omni-node (POLKADOT_NODE_VERSION).
binaries-polkadot:
    @./scripts/get_polkadot_binaries.sh polkadot-node

# Fetch / build frame-omni-bencher (driven by FRAME_OMNI_BENCHER_VERSION).
binaries-bencher:
    @./scripts/get_polkadot_binaries.sh frame-omni-bencher

# Fetch / build chain-spec-builder (driven by CHAIN_SPEC_BUILDER_VERSION).
binaries-chain-spec-builder:
    @./scripts/get_polkadot_binaries.sh chain-spec-builder

# Fetch / build try-runtime CLI (driven by TRY_RUNTIME_VERSION).
binaries-try-runtime:
    @./scripts/get_polkadot_binaries.sh try-runtime

# Fetch zombienet release binary (driven by ZOMBIENET_VERSION). Release-tag only.
binaries-zombienet:
    @./scripts/get_polkadot_binaries.sh zombienet

# Build westend-runtime WASM (RELAY_RUNTIME_VERSION). Used by zombienet runs that
# need short epochs without baking `--features fast-runtime` into the polkadot node.
binaries-relay-runtime:
    @./scripts/get_polkadot_binaries.sh relay-runtime

# Cold-cache convenience: fetch every group.
binaries-all: binaries-polkadot binaries-bencher binaries-chain-spec-builder binaries-try-runtime binaries-zombienet binaries-relay-runtime

# ---------------------------------------------------------------------------
# Chain spec generation
# ---------------------------------------------------------------------------

# Build bulletin-<runtime>-runtime + emit zombienet/bulletin-<runtime>-spec.json. runtime ∈ westend | paseo.
chain-spec runtime="westend":
    #!/usr/bin/env bash
    set -euo pipefail
    CSB_DIR="$(just binaries-chain-spec-builder)"
    export PATH="$CSB_DIR:$PATH"
    ./scripts/create_bulletin_{{runtime}}_spec.sh

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

# Pallet unit tests.
test-pallets:
    cargo test --release -p pallet-bulletin-transaction-storage

# Zombienet auto-renew suite. runtime ∈ westend | paseo.
# group selects the test slice (groups are named after the collator's chain-mode setting):
#   all      — every test (skips long-running soak); default
#   archive  — collator in archive mode, RP=10 (shared network)
#   pruning  — collator with --blocks-pruning=15, RP=10 (shared network)
#   restart  — scenarios that change pruning args across collator restarts
#   mixed    — heterogeneous standalone tests (each spawns its own network)
#   <substr> — any other value is treated as a cargo-test substring filter
test-zombienet-auto-renew runtime="westend" group="all":
    #!/usr/bin/env bash
    set -euo pipefail
    POLKADOT_BIN_DIR="$(just binaries-polkadot)"
    CSB_DIR="$(just binaries-chain-spec-builder)"
    export PATH="$CSB_DIR:$PATH"
    ./scripts/create_bulletin_{{runtime}}_spec.sh
    ./scripts/create_westend_local_spec.sh
    export ZOMBIE_PROVIDER=native
    export POLKADOT_RELAY_BINARY_PATH="$POLKADOT_BIN_DIR/polkadot"
    export POLKADOT_PARACHAIN_BINARY_PATH="$POLKADOT_BIN_DIR/polkadot-omni-node"
    export PARACHAIN_CHAIN_SPEC_PATH="$PWD/zombienet/bulletin-{{runtime}}-spec.json"
    export RELAY_CHAIN_SPEC_PATH="$PWD/zombienet/westend-local-spec.json"
    export PARACHAIN_CHAIN_ID="${PARACHAIN_CHAIN_ID:-bulletin-{{runtime}}}"
    declare -a filter_args
    case "{{group}}" in
        all)
            filter_args=(parachain_ --skip parachain_long_running_pruning_soak_test)
            ;;
        archive)
            filter_args=(--exact \
                auto_renew_storage::parachain_auto_renew_test \
                auto_renew_storage::parachain_auto_renew_many_items_test \
                auto_renew_storage::parachain_auto_renew_quota_exhaustion_test \
                auto_renew_storage::parachain_auto_renew_authorization_expires_mid_cycle_test)
            ;;
        pruning)
            filter_args=(--exact \
                auto_renew_storage::parachain_auto_renew_vs_no_renew_eviction_test \
                auto_renew_storage::parachain_renew_twice_within_block_with_pruning_test \
                auto_renew_storage::parachain_auto_renew_with_concurrent_store_test)
            ;;
        restart)
            filter_args=(parachain_restart_)
            ;;
        mixed)
            filter_args=(--exact \
                auto_renew_storage::parachain_check_proof_fails_under_pruning_test \
                auto_renew_storage::parachain_auto_renew_under_pruning_chain_halts_test \
                auto_renew_storage::parachain_auto_renew_many_items_worst_case_test \
                auto_renew_storage::parachain_on_initialize_cleanup_test \
                auto_renew_storage::parachain_on_initialize_no_renewals_weight_test)
            ;;
        *)
            filter_args=("{{group}}")
            ;;
    esac
    cargo test --release -p bulletin-chain-zombienet-sdk-tests \
        --features bulletin-chain-zombienet-sdk-tests/zombie-auto-renew-tests \
        -- --test-threads=1 --nocapture "${filter_args[@]}"

# Zombienet sync suite. runtime ∈ westend | paseo; filter is the cargo-test substring.
test-zombienet-sync runtime="westend" filter="parachain_sync_storage":
    #!/usr/bin/env bash
    set -euo pipefail
    POLKADOT_BIN_DIR="$(just binaries-polkadot)"
    CSB_DIR="$(just binaries-chain-spec-builder)"
    export PATH="$CSB_DIR:$PATH"
    ./scripts/create_bulletin_{{runtime}}_spec.sh
    ./scripts/create_westend_local_spec.sh
    export ZOMBIE_PROVIDER=native
    export POLKADOT_RELAY_BINARY_PATH="$POLKADOT_BIN_DIR/polkadot"
    export POLKADOT_PARACHAIN_BINARY_PATH="$POLKADOT_BIN_DIR/polkadot-omni-node"
    export PARACHAIN_CHAIN_SPEC_PATH="$PWD/zombienet/bulletin-{{runtime}}-spec.json"
    export RELAY_CHAIN_SPEC_PATH="$PWD/zombienet/westend-local-spec.json"
    export PARACHAIN_CHAIN_ID="${PARACHAIN_CHAIN_ID:-bulletin-{{runtime}}}"
    cargo test --release -p bulletin-chain-zombienet-sdk-tests \
        --features bulletin-chain-zombienet-sdk-tests/zombie-sync-tests \
        "{{filter}}" \
        -- --test-threads=1 --nocapture \
        --skip parachain_ldb_storage_verification_test


# ---------------------------------------------------------------------------
# Benchmarks
# ---------------------------------------------------------------------------

# Invoke frame-omni-bencher (extra args forwarded). Example: just bench v1 benchmark pallet --help
bench *args:
    #!/usr/bin/env bash
    set -euo pipefail
    BENCH_DIR="$(just binaries-bencher)"
    exec "$BENCH_DIR/frame-omni-bencher" "$@"

# ---------------------------------------------------------------------------
# Runtime migration check
# ---------------------------------------------------------------------------

# Invoke try-runtime (extra args forwarded). Example: just try-runtime on-runtime-upgrade --runtime <wasm> live --uri <wss>
try-runtime *args:
    #!/usr/bin/env bash
    set -euo pipefail
    TR_DIR="$(just binaries-try-runtime)"
    exec "$TR_DIR/try-runtime" "$@"
