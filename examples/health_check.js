import { createClient } from 'polkadot-api';
import { getWsProvider } from 'polkadot-api/ws-provider';
import { bulletin } from './.papi/descriptors/dist/index.mjs';

// Usage: node health_check.js <wss_url> [--check]
//   --check: include pallet verification (TransactionStorage)
//
// Exit codes: 0 = all OK, 1 = any FAIL, 2 = connection error
// Outputs structured JSON to stdout. Diagnostics go to stderr.

const args = process.argv.slice(2).filter(arg => !arg.startsWith('--'));
const WS_URL = args[0] || 'wss://westend-bulletin-rpc.polkadot.io';
const CHECK_PALLETS = process.argv.includes('--check');

const BLOCK_WAIT_MS = 30_000;
const CONNECT_TIMEOUT_MS = 15_000;
const METADATA_RETRIES = 10;
const METADATA_RETRY_MS = 2_000;

function log(...a) { console.error(...a); }
function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

function withTimeout(promise, ms, message) {
  return Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error(message)), ms)),
  ]);
}

async function main() {
  const result = { health: {} };
  let client;
  let exitCode = 0;

  function setFail() { if (exitCode === 0) exitCode = 1; }

  try {
    log(`Connecting to ${WS_URL}...`);
    client = createClient(getWsProvider(WS_URL));
    const typedApi = client.getTypedApi(bulletin);

    // --- RPC Connectivity ---
    const health = await withTimeout(
      client._request("system_health", []),
      CONNECT_TIMEOUT_MS,
      `Connection timed out after ${CONNECT_TIMEOUT_MS / 1000}s`,
    );
    result.health.rpc = { status: "OK", details: "Endpoint responds" };

    // --- Peers ---
    result.health.peers = {
      status: health.peers < 2 ? "WARN" : "OK",
      count: health.peers,
    };

    // --- Syncing ---
    result.health.syncing = {
      status: health.isSyncing ? "WARN" : "OK",
      isSyncing: health.isSyncing,
    };

    // --- Chain Identity (parallel RPC calls) ---
    const [chainName, nodeName, nodeVersion] = await Promise.all([
      client._request("system_chain", []),
      client._request("system_name", []),
      client._request("system_version", []),
    ]);
    result.health.chain = {
      status: "OK",
      name: chainName,
      node: nodeName,
      version: nodeVersion,
    };

    // --- Runtime Version (requires PAPI metadata) ---
    let version;
    for (let attempt = 1; attempt <= METADATA_RETRIES; attempt++) {
      try {
        version = typedApi.constants.System.Version;
        break;
      } catch (e) {
        if (attempt === METADATA_RETRIES) throw e;
        log(`  Waiting for chain metadata (${attempt}/${METADATA_RETRIES})...`);
        await sleep(METADATA_RETRY_MS);
      }
    }
    result.health.runtime = {
      status: "OK",
      specName: version.spec_name,
      specVersion: version.spec_version,
      implVersion: version.impl_version,
    };

    // --- Block Production (30s sample, retry once) ---
    log("Checking block production...");
    const header1 = await client._request("chain_getHeader", []);
    const block1 = parseInt(header1.number, 16);
    log(`  Best block: #${block1}, waiting ${BLOCK_WAIT_MS / 1000}s...`);

    await sleep(BLOCK_WAIT_MS);

    const header2 = await client._request("chain_getHeader", []);
    const block2 = parseInt(header2.number, 16);

    if (block2 > block1) {
      result.health.blockProduction = {
        status: "OK",
        from: block1,
        to: block2,
        delta: block2 - block1,
        elapsed: `${BLOCK_WAIT_MS / 1000}s`,
      };
    } else {
      log(`  No progress (#${block1} -> #${block2}), waiting another ${BLOCK_WAIT_MS / 1000}s...`);
      await sleep(BLOCK_WAIT_MS);
      const header3 = await client._request("chain_getHeader", []);
      const block3 = parseInt(header3.number, 16);
      const totalSec = (BLOCK_WAIT_MS * 2) / 1000;

      if (block3 > block1) {
        result.health.blockProduction = {
          status: "WARN",
          from: block1,
          to: block3,
          delta: block3 - block1,
          elapsed: `${totalSec}s`,
        };
      } else {
        result.health.blockProduction = {
          status: "FAIL",
          from: block1,
          to: block3,
          delta: 0,
          elapsed: `${totalSec}s`,
        };
        setFail();
      }
    }

    // --- Finalization ---
    const bestHeader = await client._request("chain_getHeader", []);
    const bestBlock = parseInt(bestHeader.number, 16);
    const finalizedHash = await client._request("chain_getFinalizedHead", []);
    const finalizedHeader = await client._request("chain_getHeader", [finalizedHash]);
    const finalizedBlock = parseInt(finalizedHeader.number, 16);
    const gap = bestBlock - finalizedBlock;

    let finStatus;
    if (gap > 100) { finStatus = "FAIL"; setFail(); }
    else if (gap > 10) { finStatus = "WARN"; }
    else { finStatus = "OK"; }

    result.health.finalization = {
      status: finStatus,
      best: bestBlock,
      finalized: finalizedBlock,
      gap,
    };

    // --- Pallet checks (--check) ---
    if (CHECK_PALLETS) {
      log("Running pallet checks...");
      result.pallet = {};

      try {
        const retentionPeriod = await typedApi.query.TransactionStorage.RetentionPeriod.getValue();
        result.pallet.retentionPeriod = {
          status: "OK",
          blocks: retentionPeriod != null ? Number(retentionPeriod) : null,
        };
      } catch (e) {
        result.pallet.retentionPeriod = { status: "FAIL", error: e.message };
        setFail();
      }

      try {
        const byteFee = await typedApi.query.TransactionStorage.ByteFee.getValue();
        result.pallet.byteFee = {
          status: "OK",
          value: byteFee != null ? byteFee.toString() : null,
        };
      } catch (e) {
        result.pallet.byteFee = { status: "FAIL", error: e.message };
        setFail();
      }

      try {
        const entryFee = await typedApi.query.TransactionStorage.EntryFee.getValue();
        result.pallet.entryFee = {
          status: "OK",
          value: entryFee != null ? entryFee.toString() : null,
        };
      } catch (e) {
        result.pallet.entryFee = { status: "FAIL", error: e.message };
        setFail();
      }

      try {
        const maxBlockTxs = typedApi.constants.TransactionStorage.MaxBlockTransactions;
        result.pallet.maxBlockTransactions = {
          status: "OK",
          value: Number(maxBlockTxs),
        };
      } catch (e) {
        result.pallet.maxBlockTransactions = { status: "FAIL", error: e.message };
        setFail();
      }

      try {
        const maxTxSize = typedApi.constants.TransactionStorage.MaxTransactionSize;
        result.pallet.maxTransactionSize = {
          status: "OK",
          value: Number(maxTxSize),
        };
      } catch (e) {
        result.pallet.maxTransactionSize = { status: "FAIL", error: e.message };
        setFail();
      }
    }
  } catch (error) {
    log(`Error: ${error.message}`);
    if (Object.keys(result.health).length === 0) {
      result.health.rpc = { status: "FAIL", details: error.message };
    }
    exitCode = 2;
  } finally {
    if (client) client.destroy();
  }

  console.log(JSON.stringify(result, null, 2));
  process.exit(exitCode);
}

await main();
