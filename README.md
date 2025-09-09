## Polkadot Bulletin chain

The Bulletin chain consists of a customized node implementation and a single runtime.

## Node implementation

The Bulletin chain node implements IPFS support on top of a regular Substrate node. Only work with `litep2p` network backend is supported (enabled by default), and in order to use IPFS functionality `--ipfs-server` flag must be passed to the node binary.

IPFS support comes in two parts:

1. Bitswap protocol implementation. Wire protocol for transferring chunks stored in transaction storage to IPFS clients. This is implemented in `litep2p` networking library and `litep2p` network backend in `sc-network` crate.
2. IPFS Kademlia DHT support. We publish content provider records for our node for CIDs (content identifiers) of transactions stored in transaction storage. Content provider records are only kept for transactions included in the chain during last two weeks, what should agree with block pruning period of the Bulletin nodes. DHT support is provided by `litep2p` networking library and `sc-network` crate. The implementation in the Bulletin node ensures we register as content providers for transactions during the last two weeks.

Bulletin node also has an idle connection timeout set to 1 hour instead of the default 10 seconds to allow manually adding the node to the swarm of an IPFS client and ensuring we don't disconnect the IPFS client. This is done to allow IPFS clients to query data over Bitswap protocol before IPFS Kademlia DHT support is implemented (DHT support is planned to be ready by the end of August 2025).

TODO: clarify if we need to store transactions for two weeks or another period.

## Runtime functionality

The Bulletin chain runtime is a standard BaBE + GRANDPA chain with a custom validator set pallet which is (currently) controlled by root call (TODO: clarify whether this should be sudo, governance, etc).
It functions to store transactions for a given period of time (currently set at 2 weeks) and provide proof of storage.

### Core functionality

The main purpose of the Bulletin chain is to provide storage for the People Chain over the bridge.

#### Storage
The core functionality of the bulletin chain is in the transaction-storage pallet, which indexes transactions and manages storage proofs for arbitrary data. 

Data is added via the `transactionStorage.store` extrinsic, provided the storage of the data is authorized by root call. Authorization is granted either for a specific account via authorize_account or for data with a specific preimage via authorize_preimage. Once data is stored, it can be retrieved from IPFS with the Blake2B hash of the data.


#### Bridge to PeopleChain
For Rococo, we have a PeopleRococo → BridgeHubRococo → Bulletin connection.

For Polkadot, the bulletin chain is bridged to directly from the proof-of-personhood chain (instead of through BridgeHub, for ease of upgrade), allowing the PoP chain to authorize preimages for storage and allowing accounts to store data.

#### PeopleChain integration
The PeopleChain root will call `transactionStorage.authorize_preimage` (over the bridge) to prime Bulletin to expect data with that hash, after which a user account will submit the data via `transactionStorage.store` (over the bridge).

### Pallets

#### polkadot-bulletin-chain/pallets/relayer-set
Controls the authorized relayers between Bulletin and PoP-polkadot.

####  polkadot-bulletin-chain/pallets/validator-set
Controls the validator set. Currently set in genesis and validators can be added and removed by root.

####  polkadot-bulletin-chain/pallets/transaction-storage
Stores arbitrary data on IPFS via the `store` extrinsic, provided that either the signer or the preimage of the data are pre-authorized. Stored data can be retrieved from IPFS or directly from the node via the transaction index or hash.

## Polkadot Bulletin production/live runtime

### Initial genesis chain spec

[bulletin-polkadot-genesis.json](./node/chain-specs/bulletin-polkadot.json)

```
cargo build --release -p polkadot-bulletin-chain

./target/release/polkadot-bulletin-chain build-spec --chain bulletin-polkadot
or
./target/release/polkadot-bulletin-chain build-spec --chain bulletin-polkadot --raw
```

### Run local chain
```
cargo build --release -p polkadot-bulletin-chain

POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain zombienet -p native spawn ./zombienet/bulletin-polkadot-local.toml
```

### Run a production chain (but only with Alice validator)
You can override the Alice validator keys here: [adjust\_bp\_spec.sh](./zombienet/adjust_bp_spec.sh) (you should see finalized blocks in the logs).

```
cargo build --release -p polkadot-bulletin-chain

POLKADOT_BULLETIN_BINARY_PATH=./target/release/polkadot-bulletin-chain ENV_PATH=<path-to-zombienet-dir-in-bulletin-repo> zombienet -p native spawn ./zombienet/bulletin-polkadot.toml
```

### Run a production chain

### Prepare keys for a production chain

**Prerequisites:**
```
# Build the node
cargo build --release -p polkadot-bulletin-chain

# Working dir (can be customized)
mkdir /tmp/bulletin
```

**Generate a validator account**
```
./target/release/polkadot-bulletin-chain key generate --scheme sr25519 --output-type json
{
  "accountId": "0x4026e944eb9c6dabc42ba6155f5a6728b1f25c93b905b082450dffc64f4b6b7b",
  "networkId": "substrate",
  "publicKey": "0x4026e944eb9c6dabc42ba6155f5a6728b1f25c93b905b082450dffc64f4b6b7b",
  "secretPhrase": "arm glove mutual frequent melt world bicycle bean later donor clown choice",
  "secretSeed": "0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646",
  "ss58Address": "5DWpUqkKHHCaRHVqgocGMnJhuvNtCfm7xvqtSd23Mu6kEVQ9",
  "ss58PublicKey": "5DWpUqkKHHCaRHVqgocGMnJhuvNtCfm7xvqtSd23Mu6kEVQ9"
}
```

**Generate node-key (used for networking and peerId)**
```
./target/release/polkadot-bulletin-chain key generate-node-key --chain bulletin-polkadot --base-path /tmp/bulletin
(example output)
Generating key in "/tmp/bulletin/chains/bulletin-polkadot/network/secret_ed25519" (secret key)
12D3KooWMTpYuDPNHoapmkfgJDCRe9XRcUuNzLYTgf82itZv4PZr (public key)

# Validate node key
./target/release/polkadot-bulletin-chain key inspect-node-key --file /tmp/bulletin/chains/bulletin-polkadot/network/secret_ed25519
(should print the same public key as above)
```

**Generate initial session keys for genesis chain spec**
```
# Babe (suri is `secretSeed`)
./target/release/polkadot-bulletin-chain key insert --chain bulletin-polkadot --base-path /tmp/bulletin --scheme sr25519 --key-type babe --suri 0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646
# (check the generate file name, starts with babe / 62616265, e.g.: 626162654026e944eb9c6dabc42ba6155f5a6728b1f25c93b905b082450dffc64f4b6b7b)
# (contains the secret key)
cat /tmp/bulletin/chains/bulletin-polkadot/keystore/626162654026e944eb9c6dabc42ba6155f5a6728b1f25c93b905b082450dffc64f4b6b7b
# "0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646"

# Grandpa (suri is `secretSeed`)
./target/release/polkadot-bulletin-chain key insert --chain bulletin-polkadot --base-path /tmp/bulletin --scheme ed25519 --key-type gran --suri 0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646
# (check the generate file name, starts with granpa / 6772616e, e.g.: 6772616e4026e944eb9c6dabc42ba6155f5a6728b1f25c93b905b082450dffc64f4b6b7b)
# (contains the secret key)
cat /tmp/bulletin/chains/bulletin-polkadot/keystore/6772616eddf71d1605421edfa311b8321e203b3d7cff1405eaeb891176638539e85a3d5b
# "0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646"

# Two files should be generated here:
./scripts/keystore-dump.sh /tmp/bulletin/chains/bulletin-polkadot/keystore
(example output)
Seed: 0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646
=== babe (sr25519)===
Secret Key URI `0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646` is account:
  Network ID:        substrate
  Secret seed:       0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646
  Public key (hex):  0x4026e944eb9c6dabc42ba6155f5a6728b1f25c93b905b082450dffc64f4b6b7b
  Account ID:        0x4026e944eb9c6dabc42ba6155f5a6728b1f25c93b905b082450dffc64f4b6b7b
  Public key (SS58): 5DWpUqkKHHCaRHVqgocGMnJhuvNtCfm7xvqtSd23Mu6kEVQ9
  SS58 Address:      5DWpUqkKHHCaRHVqgocGMnJhuvNtCfm7xvqtSd23Mu6kEVQ9
=== babe (ed25519)===
Secret Key URI `0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646` is account:
  Network ID:        substrate
  Secret seed:       0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646
  Public key (hex):  0xddf71d1605421edfa311b8321e203b3d7cff1405eaeb891176638539e85a3d5b
  Account ID:        0xddf71d1605421edfa311b8321e203b3d7cff1405eaeb891176638539e85a3d5b
  Public key (SS58): 5H5jr87N42Bpt36LKZxZcWS7P1ppgH5Yyf31C4LGb6PFFz9w
  SS58 Address:      5H5jr87N42Bpt36LKZxZcWS7P1ppgH5Yyf31C4LGb6PFFz9w

Seed: 0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646
=== gran (sr25519)===
Secret Key URI `0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646` is account:
  Network ID:        substrate
  Secret seed:       0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646
  Public key (hex):  0x4026e944eb9c6dabc42ba6155f5a6728b1f25c93b905b082450dffc64f4b6b7b
  Account ID:        0x4026e944eb9c6dabc42ba6155f5a6728b1f25c93b905b082450dffc64f4b6b7b
  Public key (SS58): 5DWpUqkKHHCaRHVqgocGMnJhuvNtCfm7xvqtSd23Mu6kEVQ9
  SS58 Address:      5DWpUqkKHHCaRHVqgocGMnJhuvNtCfm7xvqtSd23Mu6kEVQ9
=== gran (ed25519)===
Secret Key URI `0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646` is account:
  Network ID:        substrate
  Secret seed:       0x749a0904471df8d128b49dfeedf4081af0846b839c6eb69c536cf500e3886646
  Public key (hex):  0xddf71d1605421edfa311b8321e203b3d7cff1405eaeb891176638539e85a3d5b
  Account ID:        0xddf71d1605421edfa311b8321e203b3d7cff1405eaeb891176638539e85a3d5b
  Public key (SS58): 5H5jr87N42Bpt36LKZxZcWS7P1ppgH5Yyf31C4LGb6PFFz9w
  SS58 Address:      5H5jr87N42Bpt36LKZxZcWS7P1ppgH5Yyf31C4LGb6PFFz9w
```

**Update genesis chain spec script**
* File `./scripts/create_bulletin_polkadot_spec.sh`
* Update `.genesis.runtimeGenesis.patch.validatorSet.initialValidators` with a validator account public key (example above: `5DWpUqkKHHCaRHVqgocGMnJhuvNtCfm7xvqtSd23Mu6kEVQ9`)
* Update `genesis.runtimeGenesis.patch.session.keys` (and new element)
  * validator account public key
  * validator account public key
    * babe: <Babe public key (sr25519), e.g. 5DWpUqkKHHCaRHVqgocGMnJhuvNtCfm7xvqtSd23Mu6kEVQ9>
    * grandpa: <Grandpa public key (ed25519), e.g. 5H5jr87N42Bpt36LKZxZcWS7P1ppgH5Yyf31C4LGb6PFFz9w>
* Update `.bootNodes` (if needed) - format: `"/dns/bulletin-polkadot-node-todo.w3f.node.io/tcp/443/wss/p2p/12D3KooWCF1eA2Gap69zgXD7Df3e9DqDUsGoByocggTGejoHjK23"`
* Generate new chain spec:
   ```
   ./scripts/create_bulletin_polkadot_spec.sh ./target/production/wbuild/bulletin-polkadot-runtime/bulletin_polkadot_runtime.compact.compressed.wasm
   ```
* Run node
   ```
   # point to updated chain spec
   ./target/release/polkadot-bulletin-chain --validator --chain ./node/chain-specs/bulletin-polkadot.json --base-path /tmp/bulletin --node-key-file /tmp/bulletin/chains/bulletin-polkadot/network/secret_ed25519
   or
   # rebuild because of updated chain spec
   cargo build --release -p polkadot-bulletin-chain
   ./target/release/polkadot-bulletin-chain --validator --chain bulletin-polkadot --base-path /tmp/bulletin --node-key-file /tmp/bulletin/chains/bulletin-polkadot/network/secret_ed25519
   ```
* **You should see finalized blocks in the logs.**
* **!!! Push changes `./scripts/create_bulletin_polkadot_spec.sh` !!!**

### Fresh benchmarks

Run on the dedicated machine from the root directory:
```
python3 scripts/cmd/cmd.py bench bulletin-polkadot
```