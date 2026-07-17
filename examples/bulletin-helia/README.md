# Bulletin Helia - IPFS Data Fetcher

A small TypeScript library (with a reference CLI) for fetching data from a
Bulletin Chain node over IPFS by CID, using [Helia](https://github.com/ipfs/helia).

It is configured to fetch **only** over libp2p/bitswap from the peer(s) you
specify — no public IPFS gateway fallback, no DHT/bootstrap discovery. See
`src/ipfs.ts` for the exact Helia configuration.

A fetch fails fast: it gives up after a 3s timeout, or sooner if every
connected peer reports it does not have the block (bitswap `DoNotHave`).

## Prerequisites

1. **Run a bulletin chain node** with the IPFS server enabled, and note its P2P
   address, e.g. `/ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooW...`
2. **Upload some data and note its CID.**

## Installation

```bash
npm install
```

`npm install` runs the `prepare` script (`npm run build`), which compiles the
library and then the CLI into `dist-cli/` (with `.d.ts` type declarations).

The build is two steps — `build:lib` then `build:cli` — because the CLI is a
reference consumer: it imports the library by its package name (see below), so
the library must be built first for those imports to resolve.

## CLI usage

The CLI connects to the given peer(s) and fetches a single CID.

```bash
# development (tsx, no build step)
npm run dev:cli -- [-o <file>] <CID> <peer-multiaddr1> [peer-multiaddr2] ...

# or the compiled binary
node dist-cli/cli.js [-o <file>] <CID> <peer-multiaddr>
```

- `<CID>` (required): the Content Identifier to fetch.
- `<peer-multiaddr>` (required): one or more P2P multiaddrs to fetch from.
- `-o, --out <file>`: write the fetched raw bytes to `<file>` instead of
  printing hex/JSON to stdout.

Set `DEBUG=helia:bitswap:wantlist` to see which peer served each block.

## Use as a library / build your own UI

The web UI has been removed; the library exposes everything needed to build
your own. `src/cli.ts` is itself a reference consumer — it imports the library
through these same public entry points, not via internal paths. Three entry
points are published via `package.json` `exports`:

| Import                       | Exports                                                 |
| ---------------------------- | ------------------------------------------------------- |
| `bulletin-helia/ipfs`        | `IPFSClient`, `IPFSConfig`                              |
| `bulletin-helia/logger-base` | `BaseLogger` (abstract), `LogLevel`, `LogEntry`         |
| `bulletin-helia/logger-cli`  | `CLILogger` (a `BaseLogger` that writes to the console) |

### `IPFSClient`

```ts
import { IPFSClient } from 'bulletin-helia/ipfs';

const client = new IPFSClient({ logger, peerMultiaddrs });
await client.initialize();

// Parsed convenience result (tries JSON, falls back to raw):
const { data, isJSON, rawHex } = await client.fetchData(cid);

// Or the exact bytes, verified against the CID by bitswap:
const bytes: Uint8Array = await client.fetchRawBytes(cid);

await client.stop();
```

### Routing logs into your UI

`IPFSClient` takes a `logger` that extends `BaseLogger`. Implement `log()` and
`clear()` to push log entries wherever your UI needs them — the DOM, a state
store, a stream, etc. `CLILogger` (in `src/logger-cli.ts`) is a minimal
reference implementation.

```ts
import { BaseLogger, type LogLevel, type LogEntry } from 'bulletin-helia/logger-base';

class MyUiLogger extends BaseLogger {
  log(level: LogLevel, message: string, data?: unknown): void {
    const entry: LogEntry = { timestamp: new Date(), level, message, data };
    this.logs.push(entry);
    // ...render `entry` in your UI...
  }

  clear(): void {
    this.logs = [];
    // ...clear your UI...
  }
}
```

Pass an instance as `new IPFSClient({ logger: new MyUiLogger(), peerMultiaddrs })`.

## Layout

```
src/
├── ipfs.ts         # IPFSClient — the core fetch logic and Helia config   ┐
├── logger-base.ts  # BaseLogger + log types (subclass this for your UI)   ├─ library (build:lib)
├── logger-cli.ts   # CLILogger — console logger used by the CLI           ┘
└── cli.ts          # reference CLI consumer — imports 'bulletin-helia/*'  ── consumer (build:cli)
```

Build config: `tsconfig.json` holds the shared compiler options;
`tsconfig.lib.json` and `tsconfig.cli.json` extend it and differ only in which
files they include. Lint/format is [Biome](https://biomejs.dev) (`biome.json`).
