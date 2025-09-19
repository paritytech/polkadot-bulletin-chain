#!/usr/bin/env bash

usage() {
    echo Usage:
    echo "$1 <srtool compressed runtime path>"
    echo "e.g.: ./scripts/create_bulletin_polkadot_spec.sh ./target/production/wbuild/bulletin-polkadot-runtime/bulletin_polkadot_runtime.compact.compressed.wasm"
    exit 1
}

if [ -z "$1" ]; then
    usage
fi

set -e

rt_path=$1

echo "Generating chain spec for runtime: $rt_path"

# Ensure polkadot-bulletin-chain binary
binary="./target/release/polkadot-bulletin-chain"
if [ -f "$binary" ]; then
    echo "File $binary exists (no need to compile)."
else
    echo "File $binary does not exist. Compiling..."
    cargo build --profile production
fi
ls -lrt $binary

# build the chain spec we'll manipulate
$binary build-spec --chain bulletin-polkadot-local > chain-spec-plain.json

# convert runtime to hex
cat $rt_path | od -A n -v -t x1 |  tr -d ' \n' > rt-hex.txt

# TODO: provide bootNodes:
# "/dns/bulletin-polkadot-node-todo.w3f.node.io/tcp/443/wss/p2p/12D3KooWCF1eA2Gap69zgXD7Df3e9DqDUsGoByocggTGejoHjK23"

# TODO: provide sessionKeys
# TODO: provide validatorSet.initialValidators
# TODO: provide relayerSet.initialRelayers
# TODO: replace 14E5nqKAp3oAJcmzgZhUD2RcptBeUBScxKHgJKU4HPNcKVf3 (//Bob)

# TODO: provide bridgePolkadotGrandpa.initData (set some people-chain live header)

# replace the runtime in the spec with the given runtime and set some values to production
# Boot nodes, invulnerables, and session keys from https://github.com/paritytech/devops/issues/2847
#
# Note: This is a testnet runtime. Each invulnerable's Aura key is also used as its AccountId. This
# is not recommended in value-bearing networks.
cat chain-spec-plain.json | jq --rawfile code rt-hex.txt '.genesis.runtimeGenesis.code = ("0x" + $code)' \
    | jq '.name = "Polkadot Bulletin"' \
    | jq '.id = "bulletin-polkadot"' \
    | jq '.chainType = "Live"' \
    | jq '.bootNodes = [
        "/dns/bulletin.w3f.community/tcp/30333/ws/p2p/12D3KooWNcnUiQ1kbbgjzcL5yA1PN1jbp5xsTJXBCqJZ8nF8HTUg"
    ]' \
    | jq '.genesis.runtimeGenesis.patch.session.keys = [
            [
                "5F1icJDawo79k3WmVMv9VcES5KgnBofTxokhZdFvHhPYeBa1",
                "5F1icJDawo79k3WmVMv9VcES5KgnBofTxokhZdFvHhPYeBa1",
                    {
                        "babe": "5F1icJDawo79k3WmVMv9VcES5KgnBofTxokhZdFvHhPYeBa1",
                        "grandpa": "5EmjeC7fdUZg6zFEWkh5iVVYijTQitoAKnw2uHgiJ1dC6168"
                    }
            ]
        ]' \
    | jq '.genesis.runtimeGenesis.patch.validatorSet.initialValidators = [
            "5F1icJDawo79k3WmVMv9VcES5KgnBofTxokhZdFvHhPYeBa1"
        ]' \
    | jq '.genesis.runtimeGenesis.patch.relayerSet.initialRelayers = [
            "5DWpUqkKHHCaRHVqgocGMnJhuvNtCfm7xvqtSd23Mu6kEVQ9"
        ]' \
    | jq 'del(.genesis.runtimeGenesis.patch.bridgePolkadotGrandpa.owner)' \
    | jq 'del(.genesis.runtimeGenesis.patch.bridgePolkadotParachains.owner)' \
    | jq 'del(.genesis.runtimeGenesis.patch.bridgePolkadotMessages.owner)' \
    > edited-chain-spec-plain.json

# build a raw spec
$binary build-spec --chain edited-chain-spec-plain.json --raw > chain-spec-raw.json
cp edited-chain-spec-plain.json ./node/chain-specs/bulletin-polkadot-plain.json
cp chain-spec-raw.json ./node/chain-specs/bulletin-polkadot.json

