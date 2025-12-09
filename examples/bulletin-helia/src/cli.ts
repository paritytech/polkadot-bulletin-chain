#!/usr/bin/env node

import { IPFSClient } from './ipfs';
import { CLILogger } from './logger-cli';

async function main() {
  const args = process.argv.slice(2);

  if (args.length < 2) {
    console.error('Usage: bulletin-helia <CID> <peer-multiaddr1> [peer-multiaddr2] ...');
    console.error('');
    console.error('Examples:');
    console.error('  bulletin-helia bafyreifhj6h... /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooW...');
    console.error(
      '  bulletin-helia bafyreifhj6h... /ip4/127.0.0.1/tcp/10001/ws/p2p/12D3KooW... /ip4/127.0.0.1/tcp/10002/ws/p2p/12D3KooW...'
    );
    process.exit(1);
  }

  const cid = args[0];
  const peerMultiaddrs = args.slice(1);

  const logger = new CLILogger();

  logger.info('Bulletin Helia CLI - P2P Mode');
  logger.info(`CID: ${cid}`);
  logger.info(`Peers: ${peerMultiaddrs.length}`);
  peerMultiaddrs.forEach((addr, i) => {
    logger.debug(`  [${i + 1}] ${addr}`);
  });

  const client = new IPFSClient({
    logger,
    peerMultiaddrs,
  });

  try {
    await client.initialize();

    const result = await client.fetchData(cid);

    console.log('\n=== RAW BYTES ===');
    if (result.rawHex) {
      console.log(result.rawHex);
    } else {
      console.log('(No raw hex data available)');
    }

    if (result.isJSON) {
      console.log('\n=== PARSED JSON ===');
      console.log(JSON.stringify(result.data, null, 2));
    }

    await client.stop();
    process.exit(0);
  } catch (error) {
    logger.error('Failed to fetch data', error);
    process.exit(1);
  }
}

main();
