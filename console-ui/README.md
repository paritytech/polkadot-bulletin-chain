# Bulletin Chain Console UI

A web-based console for interacting with the Polkadot Bulletin Chain. Upload and download data, manage authorizations, and explore blocks.

## Features

- **Wallet Connection**: Connect Polkadot.js, Talisman, SubWallet, or other browser wallets
- **Data Upload**: Store data on-chain with configurable CID formats (hash algorithm, codec)
- **Data Download**: Retrieve data by CID via IPFS gateway
- **Authorization Management**: View account and preimage authorizations
- **Block Explorer**: Browse recent blocks and transactions
- **Network Selection**: Connect to local dev, Westend, or Polkadot networks

## Prerequisites

- Node.js 18+ or Bun
- A running Bulletin Chain node (for local development)
- A browser wallet extension (Polkadot.js, Talisman, etc.)

## Getting Started

### Install Dependencies

```bash
cd console-ui
npm install
# or
pnpm install
# or
bun install
```

### Development

```bash
npm run dev
```

Open http://localhost:5173 in your browser.

### Build for Production

```bash
npm run build
npm run preview
```

## Connecting to a Node

### Local Development

1. Start a local Bulletin Chain node:
   ```bash
   ./target/release/polkadot-bulletin-chain --dev --ipfs-server
   ```

2. The UI will auto-connect to `ws://localhost:10000`

### Westend Testnet

Select "Bulletin Westend" from the network dropdown. The UI connects to `wss://bulletin-westend-rpc.polkadot.io`.

### Polkadot Mainnet

Select "Bulletin Polkadot" from the network dropdown. The UI connects to `wss://bulletin-rpc.polkadot.io`.

## IPFS Gateway

For downloading data, configure the IPFS gateway URL:

- **Local**: `http://127.0.0.1:8283` (default for local node with `--ipfs-server`)
- **Public**: Configure based on your deployment

## Regenerating PAPI Descriptors

If the chain runtime changes, regenerate the TypeScript descriptors:

```bash
# Start a local node first
npm run papi:generate

# Or update existing descriptors
npm run papi:update
```

## Project Structure

```
console-ui/
├── src/
│   ├── main.tsx           # Entry point
│   ├── App.tsx            # Router and layout
│   ├── state/             # RxJS state management
│   │   ├── chain.state.ts # Chain connection
│   │   ├── wallet.state.ts# Wallet integration
│   │   └── storage.state.ts# Storage queries
│   ├── pages/             # Route pages
│   ├── components/        # Reusable components
│   ├── lib/               # Utilities (CID, IPFS)
│   └── utils/             # Helpers (formatting)
├── .papi/                 # PAPI descriptors
└── public/                # Static assets
```

## Tech Stack

- **React 19** + TypeScript
- **Vite** for bundling
- **Tailwind CSS v4** for styling
- **Radix UI** for accessible components
- **RxJS** + @react-rxjs for state management
- **polkadot-api** for chain interaction
- **multiformats** for CID handling

## License

Apache-2.0
