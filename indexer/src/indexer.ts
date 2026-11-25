import * as http from "node:http";
import { createClient } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws";
import { bulletin } from "@polkadot-api/descriptors";
import { Counter, Gauge, Registry } from "prom-client";
import { resolveNetwork } from "./networks.js";

const NETWORK   = resolveNetwork(process.env.INDEXER_NETWORK);
const PORT      = int("INDEXER_PORT", 9100);
const POLL_SEC  = int("INDEXER_POLL_SEC", 60);

const registry = new Registry();
const labels   = { network: NETWORK.id };

const lastBlock = new Gauge({
  name: "bulletin_indexer_last_finalised_block",
  help: "Highest finalised block number observed by the indexer",
  labelNames: ["network"],
  registers: [registry],
});

const stored = new Counter({
  name: "bulletin_stored_total",
  help: "TransactionStorage.Stored events",
  labelNames: ["network"],
  registers: [registry],
});

const renewed = new Counter({
  name: "bulletin_renewed_total",
  help: "TransactionStorage.Renewed events",
  labelNames: ["network"],
  registers: [registry],
});

const proofChecked = new Counter({
  name: "bulletin_proof_checked_total",
  help: "TransactionStorage.ProofChecked events (one per block when proof verifies)",
  labelNames: ["network"],
  registers: [registry],
});

const dataAutoRenewed = new Counter({
  name: "bulletin_data_auto_renewed_total",
  help: "TransactionStorage.DataAutoRenewed events",
  labelNames: ["network"],
  registers: [registry],
});

const autoRenewalFailed = new Counter({
  name: "bulletin_auto_renewal_failed_total",
  help: "TransactionStorage.AutoRenewalFailed events",
  labelNames: ["network"],
  registers: [registry],
});

const accountAuthorized = new Counter({
  name: "bulletin_account_authorized_total",
  help: "TransactionStorage.AccountAuthorized events",
  labelNames: ["network"],
  registers: [registry],
});

const accountAuthorizationRefreshed = new Counter({
  name: "bulletin_account_authorization_refreshed_total",
  help: "TransactionStorage.AccountAuthorizationRefreshed events",
  labelNames: ["network"],
  registers: [registry],
});

const expiredAccountAuthorizationRemoved = new Counter({
  name: "bulletin_expired_account_authorization_removed_total",
  help: "TransactionStorage.ExpiredAccountAuthorizationRemoved events",
  labelNames: ["network"],
  registers: [registry],
});

const permanentStorageNearCap = new Counter({
  name: "bulletin_permanent_storage_near_cap_events_total",
  help: "TransactionStorage.PermanentStorageNearCap rising-edge events (used >= 80% of cap)",
  labelNames: ["network"],
  registers: [registry],
});

const permanentStorageUsedBytes = new Gauge({
  name: "bulletin_permanent_storage_used_bytes",
  help: "Current PermanentStorageUsed value (bytes)",
  labelNames: ["network"],
  registers: [registry],
});

const permanentStorageCapBytes = new Gauge({
  name: "bulletin_permanent_storage_max_bytes",
  help: "MaxPermanentStorageSize chain constant (bytes)",
  labelNames: ["network"],
  registers: [registry],
});

const retentionPeriodBlocks = new Gauge({
  name: "bulletin_retention_period_blocks",
  help: "Current RetentionPeriod value (blocks)",
  labelNames: ["network"],
  registers: [registry],
});

let shuttingDown = false;
function installShutdownHandlers(server: http.Server, client: { destroy: () => void }): void {
  for (const sig of ["SIGINT", "SIGTERM"] as const) {
    process.on(sig, () => {
      if (shuttingDown) return;
      shuttingDown = true;
      console.log(`received ${sig}, shutting down`);
      server.close();
      try { client.destroy(); } catch { /* best-effort */ }
      setTimeout(() => process.exit(0), 200);
    });
  }
}

function int(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) return fallback;
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n <= 0) throw new Error(`invalid ${name}=${raw}`);
  return n;
}

async function main(): Promise<void> {
  console.log(`bulletin-indexer start network=${NETWORK.id} rpc=${NETWORK.rpc} port=${PORT}`);

  const client = createClient(getWsProvider(NETWORK.rpc));
  const api = client.getTypedApi(bulletin);

  permanentStorageCapBytes.labels(NETWORK.id).set(
    Number(await api.constants.TransactionStorage.MaxPermanentStorageSize()),
  );

  api.event.TransactionStorage.Stored.watch().subscribe((b) => {
    for (const _ of b.events) stored.labels(NETWORK.id).inc();
    lastBlock.labels(NETWORK.id).set(b.block.number);
  });
  api.event.TransactionStorage.Renewed.watch().subscribe((b) => {
    for (const _ of b.events) renewed.labels(NETWORK.id).inc();
  });
  api.event.TransactionStorage.ProofChecked.watch().subscribe((b) => {
    for (const _ of b.events) proofChecked.labels(NETWORK.id).inc();
  });
  api.event.TransactionStorage.DataAutoRenewed.watch().subscribe((b) => {
    for (const _ of b.events) dataAutoRenewed.labels(NETWORK.id).inc();
  });
  api.event.TransactionStorage.AutoRenewalFailed.watch().subscribe((b) => {
    for (const _ of b.events) autoRenewalFailed.labels(NETWORK.id).inc();
  });
  api.event.TransactionStorage.AccountAuthorized.watch().subscribe((b) => {
    for (const _ of b.events) accountAuthorized.labels(NETWORK.id).inc();
  });
  api.event.TransactionStorage.AccountAuthorizationRefreshed.watch().subscribe((b) => {
    for (const _ of b.events) accountAuthorizationRefreshed.labels(NETWORK.id).inc();
  });
  api.event.TransactionStorage.ExpiredAccountAuthorizationRemoved.watch().subscribe((b) => {
    for (const _ of b.events) expiredAccountAuthorizationRemoved.labels(NETWORK.id).inc();
  });
  api.event.TransactionStorage.PermanentStorageNearCap.watch().subscribe((b) => {
    for (const _ of b.events) permanentStorageNearCap.labels(NETWORK.id).inc();
  });
  api.event.TransactionStorage.PermanentStorageUsedUpdated.watch().subscribe((b) => {
    const last = b.events.at(-1);
    if (last) permanentStorageUsedBytes.labels(NETWORK.id).set(Number(last.payload.used));
  });

  const poll = setInterval(async () => {
    try {
      const used = await api.query.TransactionStorage.PermanentStorageUsed.getValue();
      permanentStorageUsedBytes.labels(NETWORK.id).set(Number(used));
      const retention = await api.query.TransactionStorage.RetentionPeriod.getValue();
      retentionPeriodBlocks.labels(NETWORK.id).set(Number(retention));
    } catch (e) {
      console.error("state poll failed:", e);
    }
  }, POLL_SEC * 1000);

  const server = http.createServer(async (req, res) => {
    if (req.url === "/metrics") {
      res.setHeader("Content-Type", registry.contentType);
      res.end(await registry.metrics());
      return;
    }
    if (req.url === "/healthz") {
      res.statusCode = 200;
      res.end("ok");
      return;
    }
    res.statusCode = 404;
    res.end("not found");
  });
  server.listen(PORT, () => console.log(`/metrics ready on :${PORT}`));

  installShutdownHandlers(server, client);
  process.on("beforeExit", () => clearInterval(poll));
}

main().catch((e) => {
  console.error("fatal:", e);
  process.exit(1);
});
