## Polkadot Bulletin chain

The Bulletin chain consists of a customized node implementation and a single runtime.

## Node implementation

TODO: (@dmitry-markin) add here some simple specific description or provided specific functionality, for example IPFS handling, specific configuration, ... (anything relevant for audit scope information)

## Runtime functionality

### Core functionality

The main purpose of the Bulletin chain is to provide storage for the People Chain over the bridge.

#### Storage
TODO: what/how is stored

#### Bridge to PeopleChain
For Rococo, we have a PeopleRococo → BridgeHubRococo → Bulletin connection.
For Polkadot, we (will) have a direct PeoplePolkadot → Bulletin bridge.

#### PeopleChain integration
TODO: describe use-cases or calls that are triggered from Bulletin

### Pallets

#### polkadot-bulletin-chain/pallets/relayer-set
TODO: add simple desc, what this pallet does and why we need it

####  polkadot-bulletin-chain/pallets/validator-set
TODO: add simple desc, what this pallet does and why we need it

####  polkadot-bulletin-chain/pallets/transaction-storage
TODO: add simple desc, what this pallet does and why we need it
