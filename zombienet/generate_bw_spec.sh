#!/usr/bin/env bash

set -e

script_dir=$(dirname "$0")

rt_path="$script_dir/../target/release/wbuild/bulletin-westend-runtime/bulletin_westend_runtime.compact.compressed.wasm"
para_id=2008

# build the chain spec we'll manipulate
chain-spec-builder --chain-spec-path "$script_dir/chain-spec-plain.json" create --runtime-wasm-path $rt_path default > /dev/null 2>&1

# convert runtime to hex
cat $rt_path | od -A n -v -t x1 | tr -d ' \n' > "$script_dir/rt-hex.txt" 2>/dev/null

# replace the runtime in the spec with the given runtime and set some values to production
# Boot nodes, invulnerables, and session keys from https://github.com/paritytech/devops/issues/2847
#
# Note: This is a testnet runtime. Each invulnerable's Aura key is also used as its AccountId. This
# is not recommended in value-bearing networks.
cat "$script_dir/chain-spec-plain.json" | jq --rawfile code "$script_dir/rt-hex.txt" '.genesis.runtimeGenesis.code = ("0x" + $code)' \
    | jq '.name = "Westend Bulletin"' \
    | jq '.id = "bulletin-westend"' \
    | jq '.chainType = "Local"' \
    | jq '.relay_chain = "westend"' \
    | jq --argjson para_id $para_id '.para_id = $para_id' \
    | jq --argjson para_id $para_id '.genesis.runtimeGenesis.config.parachainInfo.parachainId |= $para_id' \
    | jq --argjson para_id $para_id '.genesis.runtimeGenesis.config.parachainInfo.parachainId |= $para_id' \
    | jq '.genesis.runtimeGenesis.config.balances.balances = [
            [
              "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY",
              1152921504606846976
            ],
            [
              "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty",
              1152921504606846976
            ],
            [
              "5FLSigC9HGRKVhB9FiEo4Y3koPsNmBmLJbpXg2mp1hXcS59Y",
              1152921504606846976
            ]
          ]' \
    > "$script_dir/edited-chain-spec-plain.json"

# build a raw spec
$POLKADOT_PARACHAIN_BINARY build-spec --chain "$script_dir/edited-chain-spec-plain.json" --raw > "$script_dir/chain-spec-raw.json" 2>/dev/null
mv "$script_dir/edited-chain-spec-plain.json" "$script_dir/bulletin-westend-spec.json"
mv "$script_dir/chain-spec-raw.json" "$script_dir/bulletin-westend-spec-raw.json"

# build genesis data
$POLKADOT_PARACHAIN_BINARY export-genesis-state --chain "$script_dir/bulletin-westend-spec-raw.json" > "$script_dir/bulletin-westend-genesis-head-data" > /dev/null 2>&1

# build genesis wasm
$POLKADOT_PARACHAIN_BINARY export-genesis-wasm --chain "$script_dir/bulletin-westend-spec-raw.json" > "$script_dir/bulletin-westend-wasm"

# clean up useless files
rm "$script_dir/rt-hex.txt"
rm "$script_dir/chain-spec-plain.json"

cat "$script_dir/bulletin-westend-spec.json"