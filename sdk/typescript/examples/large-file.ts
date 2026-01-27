// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

/**
 * Large file example - Store large files with automatic chunking
 *
 * This example demonstrates:
 * - Storing large files (> 8 MiB) with automatic chunking
 * - Progress tracking during upload
 * - DAG-PB manifest creation for IPFS compatibility
 * - Retrieving chunk and manifest CIDs
 *
 * Usage:
 *   npm install
 *   npm run build
 *   node examples/large-file.js <file_path>
 *   node examples/large-file.js large_video.mp4
 */

import { AsyncBulletinClient, PAPITransactionSubmitter, StoreOptions } from '../dist/index.js';
import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider/node';
import { sr25519CreateDerive } from '@polkadot-labs/hdkd';
import { getPolkadotSigner } from 'polkadot-api/signer';
import { DEV_PHRASE } from '@polkadot-labs/hdkd-helpers';
import { readFile } from 'fs/promises';

async function main() {
  console.log('🚀 Bulletin SDK - Large File Example\n');

  // 1. Get file path from command line
  const filePath = process.argv[2];
  if (!filePath) {
    console.error('Usage: node large-file.js <file_path>');
    console.error('Example: node large-file.js large_video.mp4');
    process.exit(1);
  }

  console.log('📁 Reading file:', filePath);

  // 2. Read file data
  const data = await readFile(filePath);
  const sizeMB = data.length / 1048576;
  console.log(`📊 File size: ${data.length} bytes (${sizeMB.toFixed(2)} MB)\n`);

  // 3. Setup connection
  console.log('📡 Connecting to Bulletin Chain at ws://localhost:9944...');
  const wsProvider = getWsProvider('ws://localhost:9944');
  const papiClient = createClient(wsProvider);
  const api = papiClient.getTypedApi(/* your chain descriptors */);

  const keyring = sr25519CreateDerive(DEV_PHRASE);
  const signer = getPolkadotSigner(keyring.derive("//Alice"), "Alice", 42);

  console.log('✅ Connected to Bulletin Chain\n');

  // 4. Create client with custom config
  const submitter = new PAPITransactionSubmitter(api, signer);
  const client = new AsyncBulletinClient(submitter, {
    defaultChunkSize: 1024 * 1024, // 1 MiB chunks
    maxParallel: 8,
    createManifest: true,
  });

  // 5. Estimate authorization needed
  const estimate = client.estimateAuthorization(data.length);
  console.log('📋 Authorization estimate:');
  console.log('   Transactions needed:', estimate.transactions);
  console.log('   Total bytes:', estimate.bytes, `(${(estimate.bytes / 1048576).toFixed(2)} MB)\n`);

  // 6. Store with progress tracking
  console.log('⏳ Uploading with chunking and manifest creation...\n');

  let chunksCompleted = 0;
  let totalChunks = 0;

  const result = await client.storeChunked(
    data,
    undefined, // use default config
    undefined, // use default options
    (event) => {
      switch (event.type) {
        case 'chunk_started':
          if (totalChunks === 0) {
            totalChunks = event.total;
            console.log(`🔨 Starting upload of ${totalChunks} chunks...`);
          }
          break;

        case 'chunk_completed':
          chunksCompleted++;
          const progress = (chunksCompleted / event.total) * 100;
          console.log(
            `   ✅ Chunk ${chunksCompleted}/${event.total} completed (${progress.toFixed(1)}%) - ${event.cid.toString()}`
          );
          break;

        case 'chunk_failed':
          console.log(`   ❌ Chunk ${event.index + 1}/${event.total} failed:`, event.error.message);
          break;

        case 'manifest_started':
          console.log('\n📦 Creating DAG-PB manifest...');
          break;

        case 'manifest_created':
          console.log('   ✅ Manifest created:', event.cid.toString(), '\n');
          break;

        case 'completed':
          console.log('🎉 Upload complete!');
          if (event.manifestCid) {
            console.log('   Manifest included\n');
          }
          break;
      }
    }
  );

  // 7. Display results
  console.log('📊 Final Results:');
  console.log('   Total chunks:', result.numChunks);
  console.log('   Total size:', result.totalSize, 'bytes');
  console.log('   Chunk CIDs:', result.chunkCids.length, 'CIDs stored');

  if (result.manifestCid) {
    console.log('\n📦 DAG-PB Manifest:');
    console.log('   CID:', result.manifestCid.toString());
    console.log('\n💡 You can retrieve this file via IPFS using:');
    console.log('   ipfs cat', result.manifestCid.toString());
    console.log('   Or via HTTP gateway:');
    console.log('   https://ipfs.io/ipfs/' + result.manifestCid.toString());
  }

  console.log('\n🎉 Chunked upload completed successfully!');
  console.log('\n💡 Next steps:');
  console.log('   - Use the manifest CID to retrieve the full file');
  console.log('   - Access via IPFS gateway');
  console.log('   - Individual chunks are also stored and accessible');

  // Cleanup
  await papiClient.destroy();
}

main().catch(console.error);
