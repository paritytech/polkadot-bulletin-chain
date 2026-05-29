import { Tuple, type Codec } from "scale-ts";
import { decAnyMetadata, unifyMetadata } from "@polkadot-api/substrate-bindings";
import { getLookupFn, getDynamicBuilder } from "@polkadot-api/metadata-builders";
import type { PolkadotClient } from "polkadot-api";

export interface ExtrinsicDecoder {
  address: Codec<unknown> | null;
  signature: Codec<unknown> | null;
  extByVersion: Record<number, Codec<unknown[]>>;
  palletByIndex: Map<number, { name: string; callsCodec: Codec<any> | null }>;
}

export interface DecodedExtrinsic {
  pallet: string;
  call: string;
  args?: Record<string, unknown>;
  signer?: string;
  signed: boolean;
}

export async function buildExtrinsicDecoder(client: PolkadotClient): Promise<ExtrinsicDecoder> {
  const finalized = await client.getFinalizedBlock();
  const rawMeta = await client.getMetadata(finalized.hash);
  const parsed = decAnyMetadata(rawMeta);
  const unified = unifyMetadata(parsed);
  const lookup = getLookupFn(unified);
  const builder = getDynamicBuilder(lookup);

  const ext = unified.extrinsic as {
    address?: number;
    signature?: number;
    signedExtensions: Record<number, Array<{ type: number }>>;
  };

  const address = ext.address != null ? (builder.buildDefinition(ext.address) as Codec<unknown>) : null;
  const signature = ext.signature != null ? (builder.buildDefinition(ext.signature) as Codec<unknown>) : null;

  const extByVersion: Record<number, Codec<unknown[]>> = {};
  for (const [v, exts] of Object.entries(ext.signedExtensions)) {
    const codecs = exts.map((e) => builder.buildDefinition(e.type) as Codec<unknown>);
    extByVersion[Number(v)] = Tuple(...codecs) as Codec<unknown[]>;
  }

  const palletByIndex = new Map<number, { name: string; callsCodec: Codec<any> | null }>();
  for (const p of unified.pallets) {
    const callsCodec = p.calls
      ? (builder.buildDefinition(p.calls.type) as Codec<any>)
      : null;
    palletByIndex.set(p.index, { name: p.name, callsCodec });
  }

  return { address, signature, extByVersion, palletByIndex };
}

function compactLen(bytes: Uint8Array, offset: number): number {
  const mode = bytes[offset]! & 0x03;
  if (mode === 0) return 1;
  if (mode === 1) return 2;
  if (mode === 2) return 4;
  return 1 + (bytes[offset]! >> 2) + 4;
}

// scale-ts ignores a Uint8Array's byteOffset and reads from the start of the
// underlying buffer, so any input must be a fresh standalone buffer.
function freshSlice(bytes: Uint8Array, from: number, to?: number): Uint8Array {
  return new Uint8Array(bytes.slice(from, to));
}

function decodeAndAdvance<T>(codec: Codec<T>, bytes: Uint8Array, offset: number): number {
  const value = codec.dec(freshSlice(bytes, offset));
  return offset + codec.enc(value).length;
}

export function decodeExtrinsic(rawBytes: Uint8Array, dec: ExtrinsicDecoder): DecodedExtrinsic {
  let offset = compactLen(rawBytes, 0);
  const preamble = rawBytes[offset]!;
  offset += 1;

  const version = preamble & 0x1f;
  const xtType = (preamble >> 5) & 0x03;

  let signer: string | undefined;
  let signed = false;

  if (version === 4 && (preamble & 0x80) !== 0) {
    if (!dec.address || !dec.signature) throw new Error("Missing address/signature codec for signed v4");
    const addr = dec.address.dec(freshSlice(rawBytes, offset));
    signer = extractAccountId(addr);
    offset += dec.address.enc(addr).length;
    offset = decodeAndAdvance(dec.signature, rawBytes, offset);
    const ext = dec.extByVersion[0];
    if (!ext) throw new Error("No extension codec for v4");
    offset = decodeAndAdvance(ext, rawBytes, offset);
    signed = true;
  } else if (version === 5 && xtType === 2) {
    const extVersion = rawBytes[offset]!;
    offset += 1;
    const ext = dec.extByVersion[extVersion];
    if (!ext) throw new Error(`No extension codec for v5 ext_version=${extVersion}`);
    offset = decodeAndAdvance(ext, rawBytes, offset);
  } else if (
    !(version === 5 && xtType === 0) &&
    !(version === 4 && (preamble & 0x80) === 0)
  ) {
    throw new Error(`Unsupported extrinsic preamble 0x${preamble.toString(16)}`);
  }

  // The top-level RuntimeCall enum codec produced by PAPI's dynamic builder
  // chokes on sparse pallet indices for some metadata (variants resolve to
  // the wrong `inner[tag]`). Decode per-pallet instead: first byte is the
  // pallet index, then the pallet's own Calls enum.
  const palletIdx = rawBytes[offset]!;
  const entry = dec.palletByIndex.get(palletIdx);
  if (!entry) throw new Error(`Unknown pallet index ${palletIdx}`);
  if (!entry.callsCodec) {
    throw new Error(`Pallet ${entry.name} has no callable extrinsics in metadata`);
  }
  const decodedCall = entry.callsCodec.dec(freshSlice(rawBytes, offset + 1));
  return {
    pallet: entry.name,
    call: decodedCall.type,
    args: decodedCall.value as Record<string, unknown> | undefined,
    signer,
    signed,
  };
}

function extractAccountId(addr: unknown): string | undefined {
  if (addr && typeof addr === "object" && "type" in addr && "value" in addr) {
    const a = addr as { type: string; value: unknown };
    if (a.type === "Id" && a.value instanceof Uint8Array) {
      return "0x" + Array.from(a.value).map((b) => b.toString(16).padStart(2, "0")).join("");
    }
  }
  if (addr instanceof Uint8Array && addr.length === 32) {
    return "0x" + Array.from(addr).map((b) => b.toString(16).padStart(2, "0")).join("");
  }
  return undefined;
}
