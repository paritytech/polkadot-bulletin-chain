# Bulletin Helia - IPFS Data Fetcher

A tool for downloading and displaying data from Bulletin Chain via IPFS by CID (Content Identifier). Built with Helia, the TypeScript implementation of IPFS.

Available as both a **CLI application** and a **web application**.

## Prerequisites

1. **Run local bulletin chain** with IPFS server enabled via instruction [here](https://github.com/paritytech/polkadot-bulletin-chain/tree/main/examples#run-bulletin-solochain-with---ipfs-server) and **Note** the P2P address of the node eg. `/ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooWQCkBm1BYtkHpocxCwMgR8yjitEeHGx8spzcDLGt2gkBm` (the address is displayed within zombienet logs)

2. **Upload some data and note CID** - you can use the script available in this repo [here](https://github.com/paritytech/polkadot-bulletin-chain/tree/main/examples#using-modern-papi-polkadot-api)


## Installation

```bash
# Install dependencies
npm install

# Development
npm run dev          # Start web app dev server
npm run dev:cli      # Run CLI in dev mode
```

## CLI Usage

The CLI tool fetches raw bytes from IPFS and prints them to stdout.

```bash
# Using tsx (development)
npm run dev:cli <CID> [peer-multiaddr1] [peer-multiaddr2] ...

# Examples:
npm run dev:cli bafyreifhj6h...
npm run dev:cli bafyreifhj6h... /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooW...
```

### CLI Options

- **CID** (required): The Content Identifier to fetch
- **peer-multiaddr** (required): One or more P2P multiaddrs to connect to

## Web App Usage

```bash
npm run dev          # Start web app dev server
```

1. **Open the application** in your browser (default: http://localhost:5173)

2. **Configure connection**:
     - Enter peer multiaddrs

3. **Enter a CID** of a JSON file (see examples below)

4. **Click "Fetch JSON"** to download and display the content


