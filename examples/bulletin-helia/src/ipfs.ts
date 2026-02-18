import { createHelia, type Helia } from 'helia';
import { CID } from 'multiformats/cid';
import { multiaddr } from '@multiformats/multiaddr';
import { blake2b256 } from '@multiformats/blake2/blake2b';
import { BaseLogger } from './logger-base';

export interface IPFSConfig {
  logger: BaseLogger;
  peerMultiaddrs: string[];
}

export class IPFSClient {
  private config: IPFSConfig;
  private helia?: Helia;

  constructor(config: IPFSConfig) {
    this.config = config;
  }

  async initialize(): Promise<void> {
    this.config.logger.info('Initializing IPFS P2P client...', {
      peerMultiaddrs: this.config.peerMultiaddrs,
    });

    try {
      await this.initializeHeliaP2P();
      this.config.logger.success('IPFS client initialized successfully');
    } catch (error) {
      this.config.logger.error('Failed to initialize IPFS client', error);
      throw error;
    }
  }

  private async initializeHeliaP2P(): Promise<void> {
    this.config.logger.debug('Creating full Helia P2P node with libp2p...');
    this.config.logger.info('This will start a full IPFS node with libp2p networking');

    // Extract peer IDs from provided multiaddrs
    const allowedPeerIds = new Set<string>();
    for (const addr of this.config.peerMultiaddrs) {
      try {
        // Extract peer ID from multiaddr string (after /p2p/)
        const match = addr.match(/\/p2p\/([^/]+)/);
        if (match && match[1]) {
          const peerId = match[1];
          allowedPeerIds.add(peerId);
          this.config.logger.debug(`Whitelisted peer: ${peerId}`);
        } else {
          this.config.logger.warning(`No peer ID found in multiaddr: ${addr}`);
        }
      } catch (error) {
        this.config.logger.warning(`Failed to parse multiaddr: ${addr}`, error);
      }
    }

    this.config.logger.info(
      `Connection gater: Only allowing ${allowedPeerIds.size} whitelisted peer(s)`
    );

    // Create full Helia node with P2P capabilities
    // Configure to ONLY allow connections to specified peers
    // Include blake2b-256 hasher for Polkadot/Substrate compatibility
    this.helia = await createHelia({
      hashers: [blake2b256],
      libp2p: {
        connectionGater: {
          // Only allow connections to whitelisted peers
          denyDialMultiaddr: async maAddr => {
            const addr = maAddr.toString();

            // Extract peer ID from the address (after /p2p/)
            const match = addr.match(/\/p2p\/([^/]+)/);
            if (match && match[1]) {
              const peerId = match[1];
              if (allowedPeerIds.has(peerId)) {
                this.config.logger.debug(`Allowing whitelisted peer: ${addr}`);
                return false; // false = don't deny = allow
              }
            }

            // Deny everything else
            this.config.logger.warning(`Blocking non-whitelisted connection: ${addr}`);
            return true; // true = deny
          },
        },
      },
    });

    const peerId = this.helia.libp2p.peerId.toString();
    this.config.logger.success('Helia P2P node created', {
      peerId: peerId,
    });

    // Log multiaddrs
    const multiaddrs = this.helia.libp2p.getMultiaddrs().map(ma => ma.toString());
    this.config.logger.network('Node listening on multiaddrs', {
      count: multiaddrs.length,
      addresses: multiaddrs,
    });

    // Connect to specified peers
    this.config.logger.info(`Connecting to ${this.config.peerMultiaddrs.length} peer(s)...`);

    for (const addr of this.config.peerMultiaddrs) {
      try {
        this.config.logger.debug(`Dialing peer: ${addr}`);
        const ma = multiaddr(addr);
        await this.helia.libp2p.dial(ma);
        this.config.logger.success(`Connected to peer: ${addr}`);
      } catch (error) {
        this.config.logger.error(`Failed to connect to peer: ${addr}`, error);
        // Continue with other peers even if one fails
      }
    }

    // Log current connections
    const connections = this.helia.libp2p.getConnections();
    this.config.logger.network('Active connections', {
      count: connections.length,
      connections: connections.map(conn => ({
        peer: conn.remotePeer.toString(),
        remoteAddr: conn.remoteAddr.toString(),
        direction: conn.direction,
      })),
    });
  }

  async fetchData(cidString: string): Promise<{ data: any; isJSON: boolean; rawHex?: string }> {
    this.config.logger.info(`Starting fetch for CID: ${cidString}`);
    this.config.logger.network('Parsing CID...');

    let cid: CID;
    try {
      cid = CID.parse(cidString);

      // Determine codec name
      const codecName = this.getCodecName(cid.code);

      this.config.logger.debug('CID parsed successfully', {
        version: cid.version,
        codec: `${codecName} (0x${cid.code.toString(16)})`,
        multihash: cid.multihash.toString(),
      });

      this.config.logger.info(`Detected codec: ${codecName}`);
    } catch (error) {
      this.config.logger.error('Invalid CID format', error);
      throw new Error(`Invalid CID: ${error instanceof Error ? error.message : String(error)}`);
    }

    try {
      const result = await this.fetchViaHelia(cid);

      if (result.isJSON) {
        this.config.logger.success('Data fetched and parsed as JSON successfully');
        this.config.logger.debug('JSON preview', {
          type: typeof result.data,
          keys: typeof result.data === 'object' ? Object.keys(result.data) : undefined,
        });
      } else {
        this.config.logger.success('Data fetched successfully (raw bytes)');
        this.config.logger.debug('Raw data info', {
          hexLength: result.rawHex?.length,
          bytes: result.rawHex ? result.rawHex.length / 2 : 0,
        });
      }

      return result;
    } catch (error) {
      this.config.logger.error('Failed to fetch data', error);
      throw error;
    }
  }

  private bytesToHex(bytes: Uint8Array): string {
    return Array.from(bytes)
      .map(b => b.toString(16).padStart(2, '0'))
      .join('');
  }

  private tryParseJSON(text: string): { success: boolean; data?: any } {
    try {
      const data = JSON.parse(text);
      return { success: true, data };
    } catch {
      return { success: false };
    }
  }

  private async fetchViaHelia(cid: CID): Promise<{ data: any; isJSON: boolean; rawHex?: string }> {
    if (!this.helia) {
      throw new Error('Helia not initialized');
    }

    this.config.logger.network('Fetching via Helia...');
    this.config.logger.debug('Requesting data from IPFS network...');

    const codecName = this.getCodecName(cid.code);
    this.config.logger.info(`Codec detected: ${codecName} - Fetching raw bytes from blockstore`);

    try {
      // Just get raw bytes directly from blockstore, regardless of codec
      this.config.logger.debug('Requesting block from blockstore...');
      const blockData = await this.helia.blockstore.get(cid);

      // Log raw data
      this.config.logger.debug('📦 raw data received from ipfs', {
        type: typeof blockData,
        constructor: blockData?.constructor?.name,
        isUint8Array: blockData instanceof Uint8Array,
        length: blockData?.length,
        isBuffer: typeof Buffer !== 'undefined' && Buffer.isBuffer?.(blockData),
        toString: blockData?.toString?.(),
        isAsyncIterable: Symbol.asyncIterator in Object(blockData),
      });

      // Convert to Uint8Array if needed
      let block: Uint8Array;

      if (blockData instanceof Uint8Array) {
        block = blockData;
      } else if (typeof Buffer !== 'undefined' && Buffer.isBuffer(blockData)) {
        block = new Uint8Array(blockData);
      } else if (typeof blockData === 'object' && Symbol.asyncIterator in Object(blockData)) {
        this.config.logger.debug('Blockdata is async iterable, consuming chunks...');
        const chunks: Uint8Array[] = [];

        const timeoutMs = 30000; // 30 seconds
        const timeoutPromise = new Promise<never>((_, reject) => {
          setTimeout(() => {
            reject(
              new Error(
                `Timeout after ${timeoutMs / 1000}s waiting for data. The CID may not be available on the connected peers.`
              )
            );
          }, timeoutMs);
        });

        try {
          const iterator = (blockData as AsyncIterable<Uint8Array>)[Symbol.asyncIterator]();
          let done = false;

          while (!done) {
            const nextPromise = iterator.next();
            const result = await Promise.race([nextPromise, timeoutPromise]);

            if (result.done) {
              done = true;
            } else {
              chunks.push(result.value);
              this.config.logger.debug(`Received chunk: ${result.value.length} bytes`);
            }
          }
        } catch (error) {
          this.config.logger.error('Error consuming async iterable', error);
          throw error;
        }

        if (chunks.length === 0) {
          this.config.logger.error('No chunks received from async iterable');
          throw new Error('Block not found - the peer may not have this CID');
        }

        // Combine chunks
        const totalLength = chunks.reduce((acc, chunk) => acc + chunk.length, 0);
        block = new Uint8Array(totalLength);
        let offset = 0;
        for (const chunk of chunks) {
          block.set(chunk, offset);
          offset += chunk.length;
        }

        this.config.logger.debug(`Combined ${chunks.length} chunks into ${block.length} bytes`);
      } else if (typeof blockData === 'object' && blockData.length !== undefined) {
        // Array-like object
        block = new Uint8Array(blockData);
      } else {
        this.config.logger.error('Unknown block data type', {
          type: typeof blockData,
          constructor: blockData?.constructor?.name,
        });
        throw new Error(`Unexpected block data type: ${typeof blockData}`);
      }

      if (block.length === 0) {
        this.config.logger.error('Block has zero length after conversion');
        throw new Error('Block is empty - the peer may not have this CID');
      }

      const rawHex = this.bytesToHex(block);

      this.config.logger.debug('📦 raw data as hexstring (first 100 chars)', {
        hex: rawHex.substring(0, 100) + (rawHex.length > 100 ? '...' : ''),
        totalBytes: block.length,
      });

      this.config.logger.network('Raw block data received from Helia', {
        bytes: block.length,
      });

      // Try to decode as text and parse as JSON
      const decoder = new TextDecoder();
      const text = decoder.decode(block);
      const jsonResult = this.tryParseJSON(text);

      if (jsonResult.success) {
        this.config.logger.debug('Raw block contains valid JSON');
        return { data: jsonResult.data, isJSON: true, rawHex };
      } else {
        this.config.logger.debug('Raw block is not JSON, displaying as hex');
        return { data: text, isJSON: false, rawHex };
      }
    } catch (error) {
      this.config.logger.error('Failed to fetch raw block', error);
      throw error;
    }
  }

  async stop(): Promise<void> {
    this.config.logger.info('Stopping IPFS client...');

    try {
      if (this.helia) {
        await this.helia.stop();
        this.config.logger.debug('Helia instance stopped');
      }

      this.helia = undefined;

      this.config.logger.success('IPFS client stopped');
    } catch (error) {
      this.config.logger.error('Error stopping IPFS client', error);
      throw error;
    }
  }

  isInitialized(): boolean {
    return !!this.helia;
  }

  private getCodecName(code: number): string {
    // Common IPFS codec codes
    // See: https://github.com/multiformats/multicodec/blob/master/table.csv
    const codecs: { [key: number]: string } = {
      0x55: 'raw',
      0x70: 'dag-pb',
      0x71: 'dag-cbor',
      0x0129: 'dag-json',
      0x0297: 'dag-jose',
      0x12: 'sha2-256',
      0x85: 'json',
    };

    return codecs[code] || `unknown (0x${code.toString(16)})`;
  }
}
