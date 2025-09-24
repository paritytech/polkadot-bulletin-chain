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
binary="./target/production/polkadot-bulletin-chain"
if [ -f "$binary" ]; then
    echo "File $binary exists (no need to compile)."
else
    echo "File $binary does not exist. Compiling..."
    cargo build --profile production -p polkadot-bulletin-chain
fi
# Ensure fresh bulletin-polkadot-runtime wasm
if [ -f "$rt_path" ]; then
    echo "File $rt_path exists (let's remove and recompile)."
    rm $rt_path
else
    echo "File $rt_path does not exist. Compiling..."
fi
cargo build --profile production -p bulletin-polkadot-runtime --features on-chain-release-build

ls -lrt $binary
ls -lrt $rt_path

# build the chain spec we'll manipulate
$binary build-spec --chain bulletin-polkadot-local > chain-spec-plain.json

# convert runtime to hex
cat $rt_path | od -A n -v -t x1 |  tr -d ' \n' > rt-hex.txt

# Based on: https://github.com/paritytech/polkadot-bulletin-chain/pull/50

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
        "/dns/bulletin.w3f.community/tcp/30333/ws/p2p/12D3KooWNcnUiQ1kbbgjzcL5yA1PN1jbp5xsTJXBCqJZ8nF8HTUg",
        "/dns/bulletin.w3f.community/tcp/30333/p2p/12D3KooWNcnUiQ1kbbgjzcL5yA1PN1jbp5xsTJXBCqJZ8nF8HTUg",
        "/dns/bulletin-polkadot.bootnode.amforc.com/tcp/29999/wss/p2p/12D3KooWRdsUXZMXWV57UsBTe2oMHTekrupVVx9G1uXakFvZGHce",
        "/dns/bulletin-polkadot.bootnode.amforc.com/tcp/30044/p2p/12D3KooWRdsUXZMXWV57UsBTe2oMHTekrupVVx9G1uXakFvZGHce"
    ]' \
    | jq '.genesis.runtimeGenesis.patch.session.keys = [
            # W3F validator
            [
                "5F1icJDawo79k3WmVMv9VcES5KgnBofTxokhZdFvHhPYeBa1",
                "5F1icJDawo79k3WmVMv9VcES5KgnBofTxokhZdFvHhPYeBa1",
                    {
                        "babe": "5F1icJDawo79k3WmVMv9VcES5KgnBofTxokhZdFvHhPYeBa1",
                        "grandpa": "5EmjeC7fdUZg6zFEWkh5iVVYijTQitoAKnw2uHgiJ1dC6168"
                    }
            ],
            # Helikon.io
            [
                "5ERzYV1QjpHvL47c9hgVTczWnijR772P6FmKz9tZeQywGf7a",
                "5ERzYV1QjpHvL47c9hgVTczWnijR772P6FmKz9tZeQywGf7a",
                    {
                        "babe": "5ERzYV1QjpHvL47c9hgVTczWnijR772P6FmKz9tZeQywGf7a",
                        "grandpa": "5FNLDC8yWUsVemkhV9ehJxFDbP4Rznr348ZfkX7Ms7XcQV14"
                    }
            ],
            # Turboflakes.io
            [
                "5GjupqUGSPjfQ5bb3UFoognCR5hS35MvPkpkbeNMo85eXYAA",
                "5GjupqUGSPjfQ5bb3UFoognCR5hS35MvPkpkbeNMo85eXYAA",
                    {
                        "babe": "5FgaZokZGrDbifT83b1YV4t9Z5WdXa38AAduzDbPvGo9WJgG",
                        "grandpa": "5G7v11B1u7jQ7sBQmBza7x9kL15X5AA26VDYFCT96FkpJVdf"
                    }
            ],
            # Gatotech
            [
                "5HU4QbXWf1pbnPHdZZ4vVhGooHG1qSkVuYvqUwj4ygJbDJmL",
                "5HU4QbXWf1pbnPHdZZ4vVhGooHG1qSkVuYvqUwj4ygJbDJmL",
                    {
                        "babe": "5HU4QbXWf1pbnPHdZZ4vVhGooHG1qSkVuYvqUwj4ygJbDJmL",
                        "grandpa": "5FW2Wi2iHoMHeSGDzyCHLdqtgZayTmxPdHjbdXBWq4rdG2dm"
                    }
            ],
            # Polkadotters
            [
                "5DUuWj49HpvNdWuD9Gsa3q4zqvzcfMx73bpxpn72FP9YvXWn",
                "5DUuWj49HpvNdWuD9Gsa3q4zqvzcfMx73bpxpn72FP9YvXWn",
                    {
                        "babe": "5DUuWj49HpvNdWuD9Gsa3q4zqvzcfMx73bpxpn72FP9YvXWn",
                        "grandpa": "5GruxpZ5WJuhehJ79PuX3Dtq6C2xCVmvyY8efnrEWDiRVi3j"
                    }
            ],
            # Dwellir
            [
                "5CkCZbuP2qzwy163fm6b7Y95evqvkwHwkBf9K11E43XXFKwF",
                "5CkCZbuP2qzwy163fm6b7Y95evqvkwHwkBf9K11E43XXFKwF",
                    {
                        "babe": "5EshysGKxemEtnrtTXqQtqXQKuF5uwKJuVXYs4wQztABJ5mC",
                        "grandpa": "5FxEbEUZR7uQxi6k3Kb1Pormb4vofax3ouT5yZMs9jfoz7js"
                    }
            ],
            # Stake.Plus
            [
                "5DLCJxpSXYTRxJh7b9QFhQKr9nh3e5D298AruWGcM6tpNczM",
                "5DLCJxpSXYTRxJh7b9QFhQKr9nh3e5D298AruWGcM6tpNczM",
                    {
                        "babe": "5DLocAHf1LXJUmNpPpevLP6f7jyTw5Zx2uPbZ32t9Kr1c9ZE",
                        "grandpa": "5GxasehWwuxH1Qvxg8j2M8VvMws2Ft3nTNCQu9JdE2pB77Wf"
                    }
            ],
            # Amforc
            [
                "5EqaVoqJY6QGK4DMCef5bDDYxMdJgrPcToDE9smrkUFMNBK7",
                "5EqaVoqJY6QGK4DMCef5bDDYxMdJgrPcToDE9smrkUFMNBK7",
                    {
                        "babe": "5EqaVoqJY6QGK4DMCef5bDDYxMdJgrPcToDE9smrkUFMNBK7",
                        "grandpa": "5GfwHE99zesmMTZ9iBuQojp7WpsZmXuzf7nTspASLArfgsRB"
                    }
            ]
        ]' \
    | jq '.genesis.runtimeGenesis.patch.validatorSet.initialValidators = [
            "5F1icJDawo79k3WmVMv9VcES5KgnBofTxokhZdFvHhPYeBa1",
            "5ERzYV1QjpHvL47c9hgVTczWnijR772P6FmKz9tZeQywGf7a",
            "5GjupqUGSPjfQ5bb3UFoognCR5hS35MvPkpkbeNMo85eXYAA",
            "5HU4QbXWf1pbnPHdZZ4vVhGooHG1qSkVuYvqUwj4ygJbDJmL",
            "5DUuWj49HpvNdWuD9Gsa3q4zqvzcfMx73bpxpn72FP9YvXWn",
            "5CkCZbuP2qzwy163fm6b7Y95evqvkwHwkBf9K11E43XXFKwF",
            "5DLCJxpSXYTRxJh7b9QFhQKr9nh3e5D298AruWGcM6tpNczM",
            "5EqaVoqJY6QGK4DMCef5bDDYxMdJgrPcToDE9smrkUFMNBK7"
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

