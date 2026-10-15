// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { CidCodec, HashAlgorithm } from "@parity/bulletin-sdk";

const CODEC_NAMES: Record<number, string> = {
  [CidCodec.Raw]: "Raw",
  [CidCodec.DagPb]: "DAG-PB",
  [CidCodec.DagCbor]: "DAG-CBOR",
};

const HASH_NAMES: Record<number, string> = {
  [HashAlgorithm.Blake2b256]: "Blake2b-256",
  [HashAlgorithm.Sha2_256]: "SHA2-256",
  [HashAlgorithm.Keccak256]: "Keccak-256",
};

/** Runtime `HashingAlgorithm` variant names, as they arrive from chain metadata. */
const RUNTIME_HASH_NAMES: Record<string, string> = {
  Blake2b256: "Blake2b-256",
  Sha2_256: "SHA2-256",
  Keccak256: "Keccak-256",
};

export function hexCode(code: number): string {
  return `0x${code.toString(16)}`;
}

export function codecName(code: number): string {
  return CODEC_NAMES[code] ?? "Unknown";
}

export function hashName(code: number): string {
  return HASH_NAMES[code] ?? "Unknown";
}

/** e.g. `Raw / 0x55` */
export function codecLabel(code: number): string {
  return `${codecName(code)} / ${hexCode(code)}`;
}

/** e.g. `Blake2b-256 / 0xb220` */
export function hashLabel(code: number): string {
  return `${hashName(code)} / ${hexCode(code)}`;
}

/** Chain metadata reports the hashing algorithm as a variant name, not a multicodec code. */
export function runtimeHashName(type: string): string {
  return RUNTIME_HASH_NAMES[type] ?? type;
}
