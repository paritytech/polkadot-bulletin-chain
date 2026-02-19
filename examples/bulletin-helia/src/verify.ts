#!/usr/bin/env node

/**
 * Verify CID content via P2P.
 *
 * Fetches a CID from Bulletin nodes via direct P2P (Helia/libp2p) and outputs
 * raw bytes as a hex string to stdout. Designed to be called from test scripts.
 *
 * Usage: tsx src/verify.ts <CID> <peer-multiaddr1> [peer-multiaddr2] ...
 * Output: raw hex string on stdout (all logs go to stderr)
 * Exit:   0 on success, 1 on failure
 */

import { IPFSClient } from './ipfs';
import { BaseLogger, type LogLevel } from './logger-base';

/** Logger that writes everything to stderr so stdout stays clean for data output. */
class StderrLogger extends BaseLogger {
  log(level: LogLevel, message: string, data?: any): void {
    const entry = { timestamp: new Date(), level, message, data };
    this.logs.push(entry);

    const line = `[${this.formatTimestamp(entry.timestamp)}] [${level.toUpperCase()}] ${message}`;
    process.stderr.write(line + '\n');
    if (data) {
      process.stderr.write(
        (typeof data === 'string' ? data : JSON.stringify(data, null, 2)) + '\n'
      );
    }
  }

  clear(): void {
    this.logs = [];
  }
}

async function main() {
  const args = process.argv.slice(2);

  if (args.length < 2) {
    process.stderr.write(
      'Usage: tsx src/verify.ts <CID> <peer-multiaddr1> [peer-multiaddr2] ...\n'
    );
    process.exit(1);
  }

  const cid = args[0];
  const peerMultiaddrs = args.slice(1);
  const logger = new StderrLogger();

  logger.info(`Verifying CID: ${cid}`);
  logger.info(`Peers: ${peerMultiaddrs.length}`);

  const client = new IPFSClient({ logger, peerMultiaddrs });

  try {
    await client.initialize();
    const result = await client.fetchData(cid);
    await client.stop();

    if (!result.rawHex) {
      logger.error('No data received');
      process.exit(1);
    }

    // Output ONLY the hex string to stdout — callers parse this
    process.stdout.write(result.rawHex);
    process.exit(0);
  } catch (error) {
    logger.error('Verification failed', error);
    await client.stop().catch(() => {});
    process.exit(1);
  }
}

main();
