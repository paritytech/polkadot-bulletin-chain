import * as Sentry from "@sentry/node";
import { createClient, type PolkadotSigner } from "polkadot-api";
import { getWsProvider } from "polkadot-api/ws";
import { getPolkadotSigner } from "polkadot-api/signer";
import { sr25519CreateDerive } from "@polkadot-labs/hdkd";
import {
  DEV_PHRASE,
  entropyToMiniSecret,
  mnemonicToEntropy,
} from "@polkadot-labs/hdkd-helpers";
import { bulletin } from "@polkadot-api/descriptors";
import { randomBytes } from "node:crypto";
import { resolveNetwork, type Network } from "./networks.js";

const DSN           = required("SENTRY_DSN");
const NETWORK       = resolveNetwork(process.env.PROBE_NETWORK);
const MNEMONIC      = process.env.PROBE_MNEMONIC ?? DEV_PHRASE;
const PAYLOAD_BYTES = int("PROBE_PAYLOAD_BYTES", 64 * 1024);
const INTERVAL_SEC  = int("PROBE_INTERVAL_SEC", 300);
const TX_TIMEOUT_MS = int("PROBE_TX_TIMEOUT_SEC", 180) * 1000;

Sentry.init({
  dsn: DSN,
  release: "bulletin-probe@0.1.0",
  environment: NETWORK.id,
  tracesSampleRate: 1.0,
});

const SIGNER = buildSigner(MNEMONIC);

async function probeOnce(net: Network): Promise<void> {
  const payload  = new Uint8Array(randomBytes(PAYLOAD_BYTES));
  const carMb    = (payload.length / 1_000_000).toFixed(2);
  const probeTag = `slo-${net.id}`;

  await Sentry.startSpan(
    {
      name: "probe deploy",
      op:   "deploy",
      attributes: {
        "deploy.network": net.id,
        "deploy.probe":   probeTag,
        "deploy.tool":    "bulletin-probe@0.1.0",
      },
    },
    () =>
      Sentry.startSpan(
        {
          name: "1b. chunk-upload",
          op:   "deploy.chunk-upload",
          attributes: {
            "deploy.chunks.total":    1,
            "deploy.car.bytes":       payload.length,
            "deploy.car.mb":          carMb,
            "deploy.car.size_bucket": "tiny",
            "deploy.probe":           probeTag,
          },
        },
        () => submitFinalized(net, payload),
      ),
  );
}

async function submitFinalized(net: Network, data: Uint8Array): Promise<void> {
  const client = createClient(getWsProvider(net.rpc));
  try {
    const api = client.getTypedApi(bulletin);
    const tx  = api.tx.TransactionStorage.store({ data });
    await waitFinalized(tx, SIGNER, TX_TIMEOUT_MS);
  } finally {
    client.destroy();
  }
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
  console.log(
    `bulletin-probe start network=${NETWORK.id} rpc=${NETWORK.rpc} ` +
    `interval=${INTERVAL_SEC}s payload=${PAYLOAD_BYTES}B`,
  );
  for (;;) {
    const t0 = Date.now();
    try {
      await probeOnce(NETWORK);
      console.log(`probe ok in ${Date.now() - t0}ms`);
    } catch (e) {
      console.error(`probe failed in ${Date.now() - t0}ms:`, e);
      Sentry.captureException(e);
    }
    await new Promise((r) => setTimeout(r, INTERVAL_SEC * 1000));
  }
}

main().catch((e) => {
  console.error("fatal:", e);
  Sentry.captureException(e);
  process.exit(1);
});
