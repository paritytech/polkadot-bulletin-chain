// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Simple store example - Store small data on Bulletin Chain
 *
 * This example demonstrates:
 * - Connecting to Bulletin Chain using PAPI
 * - Creating a signer from a development account
 * - Storing data and getting back the CID
 * - Viewing the transaction receipt
 *
 * Usage:
 *   npm install
 *   npm run build
 *   node examples/simple-store.js
 */

import { AsyncBulletinClient, StoreOptions, CidCodec, HashAlgorithm } from '../dist/index.js';
import { createClient, Binary } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import { getPolkadotSigner } from 'polkadot-api/signer';
import { DEV_PHRASE } from '@polkadot-labs/hdkd-helpers';

async function main() {
  console.log('🚀 Bulletin SDK - Simple Store Example\n');

  // 1. Setup: Connect to local node
  console.log('📡 Connecting to Bulletin Chain at ws://localhost:9944...');
  const wsProvider = getWsProvider('ws://localhost:9944');
  const papiClient = createClient(wsProvider);

  // Get typed API (you would normally generate this from chain metadata)
  const api = papiClient.getTypedApi(/* your chain descriptors */);

  // 2. Create signer (using Alice's dev account)
  const keyring = sr25519CreateDerive(DEV_PHRASE);
  const signer = getPolkadotSigner(
    keyring.derive("//Alice"),
    "Alice",
    42 // Bulletin chain ID
  );

  console.log('🔑 Using account: Alice');
  console.log('✅ Connected to Bulletin Chain\n');

  // 3. Create Bulletin client (directly with PAPI client and signer)
  const client = new AsyncBulletinClient(api, signer);

  // 4. Prepare data to store using PAPI's Binary class
  const message = 'Hello, Bulletin Chain! This is a simple store example.';
  const data = Binary.fromText(message);
  console.log('📝 Data to store:', data.asBytes().length, 'bytes');
  console.log('   Content:', message, '\n');

  // 5. Store data using builder pattern
  console.log('⏳ Storing data on chain...');
  const result = await client.store(data).send();

  // 6. Display results
  console.log('✅ Data stored successfully!\n');
  console.log('📊 Results:');
  console.log('   CID:', result.cid.toString());
  console.log('   Data size:', result.size, 'bytes');
  if (result.blockNumber) {
    console.log('   Block number:', result.blockNumber);
  }

  console.log('\n🎉 Example completed successfully!');
  console.log('\n💡 Next steps:');
  console.log('   - Try the large-file example for chunked uploads');
  console.log('   - Use the CID to retrieve data via IPFS gateway');
  console.log('   - Check the authorization example for managing permissions');

  // Cleanup
  await papiClient.destroy();
}

main().catch(console.error);
