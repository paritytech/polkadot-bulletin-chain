## Polkadot Bulletin chain

The Bulletin chain consists of a customized node implementation and a single runtime.

## Node implementation

The Bulletin chain node implements IPFS support on top of a regualar Substrate node. Only work with `litep2p` network backend is supported (enabled by default), and in order to use IPFS functionality `--ipfs-server` flag must be passed to the node binary.

IPFS support comes in two parts:

1. Bitswap protocol implementation. Wire protocol for transferring chunks stored in transaction storage to IPFS clients. This is implemented in `litep2p` networking library and `litep2p` network backend in `sc-network` crate.
2. IPFS Kademlia DHT support. We publish content provider records for our node for CIDs (content identifiers) of transactions stored in transaction storage. Content provider records are only kept for transactions included in the chain during last two weeks, what should agree with block pruning period of the Bulletin nodes. DHT support is provided by `litep2p` networking library and `sc-network` crate. The implementation in the Bulletin node ensures we register as content providers for transactions during last two weeks.

Bulletin node also has idle connection timeout set to 1 hour instead of default 10 seconds to allow manually adding the node to the swarm of an IPFS client and ensuring we don't disconnect the IPFS client. This is done to allow IPFS clients to query data over Bitswap protocol before IPFS Kademlia DHT support is implemented (DHT support is planned to be ready by the end of August 2025).

TODO: clarify if we need to store transactiond for two weeks or other period.

## Runtime functionality

The Bulletin chain runtime is a standard BaBE + GRANDPA chain with a custom validator set pallet which is (currently) controlled by root call (TODO: clarify whether this should be sudo, governance, etc).
It functions to store transactions for a given period of time (currently set at 2 weeks) and provide proofs of storage.

### Core functionality

The main purpose of the Bulletin chain is to provide storage for the People Chain over the bridge.

#### Storage
The core functionality of the bulletin chain is in the transaction-storage pallet, which indexes transcations and manages storage proofs for arbitrary data. 

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

### Fresh benchmarks

Run on the dedicated machine from the root directory:
```
python3 scripts/cmd/cmd.py bench bulletin-polkadot
```