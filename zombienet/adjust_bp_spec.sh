#!/bin/bash

# Add Alice(`5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY`) as pre-defined validator
# We do this only if there is a `.genesis.runtimeGenesis.patch` object.
# Otherwise we're working with the raw chain spec.
$POLKADOT_BULLETIN_BINARY_PATH build-spec --chain bulletin-polkadot-local  \
    | jq '.name = "Polkadot Bulletin (Alice/Bob patched)"' \
    | jq '.id = "bulletin-polkadot"' \
    | jq '.chainType = "Live"' \
    | jq '.bootNodes = [
        "/ip4/127.0.0.1/tcp/33333/ws/p2p/5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
    ]' \
    | jq '.genesis.runtimeGenesis.patch.session.keys = [
            [
                "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY",
                "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY",
                    {
                        "babe": "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY",
                        "grandpa": "5FA9nQDVg267DEd8m1ZypXLBnvN7SFxYwV7ndqSYGiN9TTpu"
                    }
            ]
        ]' \
    | jq '.genesis.runtimeGenesis.patch.validatorSet.initialValidators = [
            "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
        ]' \
    | jq '.genesis.runtimeGenesis.patch.relayerSet.initialRelayers = [
            "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
        ]'
