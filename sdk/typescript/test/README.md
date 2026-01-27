# Bulletin SDK TypeScript Tests

This directory contains unit and integration tests for the Bulletin SDK TypeScript implementation.

## Test Types

### Unit Tests (`test/unit/`)

Fast, isolated tests that don't require a running node:

- **`chunker.test.ts`** - Data chunking logic
  - Chunk size validation
  - Data splitting and reassembly
  - Edge cases (empty data, partial chunks)

- **`cid.test.ts`** - CID calculation
  - Different codecs (Raw, DagPb)
  - Different hash algorithms (Blake2b, SHA2, Keccak)
  - Determinism and consistency

- **`authorization.test.ts`** - Authorization estimation
  - Transaction count calculation
  - Byte count validation
  - Custom chunk sizes

### Integration Tests (`test/integration/`)

Full end-to-end tests that connect to a running Bulletin Chain node:

- **`client.test.ts`** - AsyncBulletinClient operations
  - Store operations (simple and chunked)
  - Authorization workflows
  - Maintenance operations
  - Complete end-to-end workflows

## Prerequisites

### 1. Install Dependencies

```bash
npm install
# or
yarn install
# or
pnpm install
```

### 2. Build SDK

```bash
npm run build
```

### 3. Running Node (Integration Tests Only)

Integration tests require a local Bulletin Chain node at `ws://localhost:9944`:

```bash
# From the project root
cargo build --release
./target/release/polkadot-bulletin-chain --dev --tmp
```

## Running Tests

### All Tests

```bash
npm test
```

### Unit Tests Only

```bash
npm run test:unit
```

### Integration Tests Only

```bash
npm run test:integration
```

### Watch Mode

```bash
npm run test:watch
```

### Coverage Report

```bash
npm run test:coverage
```

## Test Configuration

Tests use Vitest with configuration in `vitest.config.ts`:

```typescript
export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    testTimeout: 30000, // 30s for integration tests
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html'],
    },
  },
});
```

## Test Output

Tests include detailed console output:

```
✅ Simple store test passed
   CID: bafkreiabcd1234...
   Size: 42 bytes

✅ Chunked store test passed
   Chunk 1/5 completed
   Chunk 2/5 completed
   ...
   Manifest created: bafybeief5678...
   Chunks: 5
   Manifest CID: bafybeief5678...

✅ Account authorization test passed
   Block hash: 0x1234abcd...
```

## Test Coverage

Current test coverage:

**Unit Tests:**
- ✅ Chunking logic
- ✅ CID calculation (all codecs and algorithms)
- ✅ Authorization estimation
- ✅ Data integrity validation
- ✅ Edge cases handling

**Integration Tests:**
- ✅ Simple store operation
- ✅ Chunked store with progress tracking
- ✅ Custom CID configurations
- ✅ Account authorization workflow
- ✅ Preimage authorization workflow
- ✅ Authorization refresh
- ✅ Data renewal
- ✅ Expired authorization cleanup
- ✅ Complete end-to-end workflows

## Writing New Tests

### Adding Unit Tests

Create a new file in `test/unit/`:

```typescript
import { describe, it, expect } from 'vitest';

describe('MyFeature', () => {
  it('should do something', () => {
    // Your test logic
    expect(actual).toBe(expected);
  });
});
```

### Adding Integration Tests

Add to `test/integration/client.test.ts`:

```typescript
describe('New Feature', () => {
  it('should test new feature', async () => {
    const result = await client.myNewFeature();
    expect(result).toBeDefined();
  });
});
```

## Troubleshooting

### Connection Failed

```
Error: Failed to connect to ws://localhost:9944
```

**Solution:** Ensure the Bulletin Chain node is running:
```bash
./target/release/polkadot-bulletin-chain --dev --tmp
```

### Module Not Found

```
Error: Cannot find module '../dist/index.js'
```

**Solution:** Build the SDK first:
```bash
npm run build
```

### Test Timeout

```
Error: Test timed out
```

**Solution:** Increase timeout in test file:
```typescript
it('long running test', async () => {
  // ...
}, 60000); // 60 seconds
```

Or update `vitest.config.ts`:
```typescript
testTimeout: 60000
```

### Authorization Failed

```
Error: InsufficientAuthorization
```

**Solution:** Tests use Alice's account which should have sudo. Ensure the node is running in `--dev` mode.

## CI/CD Integration

Example GitHub Actions workflow:

```yaml
name: Test

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest

    steps:
      - uses: actions/checkout@v3

      - name: Setup Node.js
        uses: actions/setup-node@v3
        with:
          node-version: '18'

      - name: Install dependencies
        run: |
          cd sdk/typescript
          npm install

      - name: Run unit tests
        run: |
          cd sdk/typescript
          npm run test:unit

      - name: Start Bulletin Chain
        run: |
          cargo build --release
          ./target/release/polkadot-bulletin-chain --dev --tmp &
          sleep 10

      - name: Run integration tests
        run: |
          cd sdk/typescript
          npm run test:integration

      - name: Upload coverage
        uses: codecov/codecov-action@v3
        with:
          files: ./sdk/typescript/coverage/coverage-final.json
```

## Test Utilities

### Mock Data Generation

```typescript
function generateTestData(size: number): Uint8Array {
  return new Uint8Array(size).fill(0x42);
}

function generateRandomData(size: number): Uint8Array {
  const data = new Uint8Array(size);
  for (let i = 0; i < size; i++) {
    data[i] = Math.floor(Math.random() * 256);
  }
  return data;
}
```

### Async Helpers

```typescript
async function waitForBlocks(count: number): Promise<void> {
  await new Promise(resolve => setTimeout(resolve, count * 6000));
}
```

## Performance Testing

Some tests include performance metrics:

```typescript
const start = Date.now();
const result = await client.storeChunked(largeData);
const duration = Date.now() - start;

console.log(`   Upload time: ${duration}ms`);
console.log(`   Throughput: ${(largeData.length / duration * 1000 / 1048576).toFixed(2)} MB/s`);
```

## Debugging Tests

### Run Single Test

```bash
npm test -- chunker.test.ts
```

### Run with Debug Output

```bash
DEBUG=* npm test
```

### VS Code Debug Configuration

Add to `.vscode/launch.json`:

```json
{
  "type": "node",
  "request": "launch",
  "name": "Debug Tests",
  "runtimeExecutable": "npm",
  "runtimeArgs": ["test"],
  "console": "integratedTerminal",
  "internalConsoleOptions": "neverOpen"
}
```
