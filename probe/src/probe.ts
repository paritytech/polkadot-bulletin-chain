import * as Sentry from "@sentry/node";
import {
  Binary,
  createClient,
  type PolkadotSigner,
  type TypedApi,
} from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws";
import { getPolkadotSigner } from "polkadot-api/signer";
import { sr25519CreateDerive } from "@polkadot-labs/hdkd";
import {
  DEV_PHRASE,
  entropyToMiniSecret,
  mnemonicToEntropy,
} from "@polkadot-labs/hdkd-helpers";
import { blake2b } from "@noble/hashes/blake2.js";
import { bulletin } from "@polkadot-api/descriptors";

type BulletinApi = TypedApi<typeof bulletin>;
import { randomBytes } from "node:crypto";
import { resolveNetwork, type Network } from "./networks.js";
import { closeSentry, initSentry } from "./sentry.js";

const RELEASE       = "bulletin-probe@0.1.0";
const FLUSH_MS      = 5_000;
const DSN           = required("SENTRY_DSN");
const NETWORK       = resolveNetwork(process.env.PROBE_NETWORK);
const MNEMONIC      = process.env.PROBE_MNEMONIC ?? DEV_PHRASE;
const PAYLOAD_BYTES = int("PROBE_PAYLOAD_BYTES", 64 * 1024);
const INTERVAL_SEC  = int("PROBE_INTERVAL_SEC", 300);
const TX_TIMEOUT_MS = int("PROBE_TX_TIMEOUT_SEC", 180) * 1000;
const RD_TIMEOUT_MS = int("PROBE_READ_TIMEOUT_SEC", 10) * 1000;

initSentry({ dsn: DSN, release: RELEASE, environment: NETWORK.id });

const SIGNER = buildSigner(MNEMONIC);

let shuttingDown = false;
function installShutdownHandlers(): void {
  for (const sig of ["SIGINT", "SIGTERM"] as const) {
    process.on(sig, async () => {
      if (shuttingDown) return;
      shuttingDown = true;
      console.log(`received ${sig}, flushing sentry…`);
      await closeSentry(FLUSH_MS);
      process.exit(0);
    });
  }
}

async function probeOnce(net: Network): Promise<void> {
  const payload     = new Uint8Array(randomBytes(PAYLOAD_BYTES));
  const contentHash = blake2b(payload, { dkLen: 32 });
  const client      = createClient(getWsProvider(net.rpc));
  try {
    const api = client.getTypedApi(bulletin);
    await probeWrite(api, net, payload);
    await probeRead(api, net, contentHash);
  } finally {
    client.destroy();
  }
}

function probeWrite(api: BulletinApi, net: Network, payload: Uint8Array): Promise<void> {
  return Sentry.startSpan(
    {
      name: "store one chunk",
      op:   "probe.bulletin.store",
      attributes: {
        "probe.network":         net.id,
        "probe.payload_bytes":   payload.length,
        "probe.chunks":          1,
        "probe.tool_version":    RELEASE,
        // Seeded denominators so Sentry ratio queries work; flipped to "true"
        // when the matching failure mode is hit.
        "probe.tx_timeout":      "false",
        "probe.tx_dropped":      "false",
      },
    },
    async (span) => {
      const tx = api.tx.TransactionStorage.store({ data: payload });
      try {
        await waitFinalized(tx, SIGNER, TX_TIMEOUT_MS);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        if (msg.includes("not finalized within")) span.setAttribute("probe.tx_timeout", "true");
        if (msg.includes("dropped"))               span.setAttribute("probe.tx_dropped", "true");
        throw e;
      }
    },
  );
}

function probeRead(api: BulletinApi, net: Network, contentHash: Uint8Array): Promise<void> {
  return Sentry.startSpan(
    {
      name: "query by content hash",
      op:   "probe.bulletin.read",
      attributes: {
        "probe.network":      net.id,
        "probe.tool_version": RELEASE,
        // Seeded denominators.
        "probe.read_miss":    "false",
        "probe.read_timeout": "false",
      },
    },
    async (span) => {
      const hashHex = Binary.toHex(contentHash);
      try {
        const result = await Promise.race([
          api.query.TransactionStorage.TransactionByContentHash.getValue(hashHex),
          new Promise<never>((_, reject) =>
            setTimeout(() => reject(new Error(`read not returned within ${RD_TIMEOUT_MS}ms`)), RD_TIMEOUT_MS),
          ),
        ]);
        if (!result) {
          span.setAttribute("probe.read_miss", "true");
          throw new Error("content hash absent from TransactionByContentHash");
        }
      } catch (e) {
        if (e instanceof Error && e.message.includes("read not returned")) {
          span.setAttribute("probe.read_timeout", "true");
        }
        throw e;
      }
    },
  );
}

function waitFinalized(
  tx: { signSubmitAndWatch: (signer: PolkadotSigner) => { subscribe: (o: any) => { unsubscribe: () => void } } },
  signer: PolkadotSigner,
  timeoutMs: number,
): Promise<void> {
  return new Promise((resolve, reject) => {
    let sub: { unsubscribe: () => void } | undefined;
    let done = false;
    const timer = setTimeout(() => {
      if (done) return;
      done = true;
      sub?.unsubscribe();
      reject(new Error(`probe tx not finalized within ${timeoutMs}ms`));
    }, timeoutMs);

    sub = tx.signSubmitAndWatch(signer).subscribe({
      next: (ev: any) => {
        if (done) return;
        if (ev.type === "finalized") {
          done = true;
          clearTimeout(timer);
          if (ev.ok === false) {
            reject(new Error(`probe tx failed: ${JSON.stringify(ev.dispatchError)}`));
          } else {
            resolve();
          }
        }
      },
      error: (err: unknown) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        reject(err instanceof Error ? err : new Error(String(err)));
      },
    });
  });
}

function buildSigner(mnemonic: string): PolkadotSigner {
  const entropy = mnemonicToEntropy(mnemonic);
  const mini    = entropyToMiniSecret(entropy);
  const derive  = sr25519CreateDerive(mini);
  const kp      = derive("");
  return getPolkadotSigner(kp.publicKey, "Sr25519", kp.sign);
}

function required(name: string): string {
  const v = process.env[name];
  if (!v) {
    console.error(`missing required env var ${name}`);
    process.exit(2);
  }
  return v;
}

function int(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) return fallback;
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n) || n <= 0) {
    throw new Error(`invalid ${name}=${raw}`);
  }
  return n;
}

async function main(): Promise<void> {
  installShutdownHandlers();
  console.log(
    `bulletin-probe start network=${NETWORK.id} rpc=${NETWORK.rpc} ` +
    `interval=${INTERVAL_SEC}s payload=${PAYLOAD_BYTES}B`,
  );
  while (!shuttingDown) {
    const t0 = Date.now();
    try {
      await probeOnce(NETWORK);
      console.log(`probe ok in ${Date.now() - t0}ms`);
    } catch (e) {
      console.error(`probe failed in ${Date.now() - t0}ms:`, e);
      Sentry.captureException(e);
    }
    if (shuttingDown) break;
    await new Promise((r) => setTimeout(r, INTERVAL_SEC * 1000));
  }
}

main().catch(async (e) => {
  console.error("fatal:", e);
  Sentry.captureException(e);
  await closeSentry(FLUSH_MS);
  process.exit(1);
});
