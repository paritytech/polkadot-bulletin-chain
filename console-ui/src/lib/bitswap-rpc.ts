// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import type { PolkadotClient } from "polkadot-api";
import { hexToBytes } from "@/utils/format";

/**
 * Fetch a stored block from the connected node via the `bitswap_v1_get`
 * JSON-RPC method. Returns the raw indexed transaction data — no DAG
 * assembly, so dag-pb root CIDs yield the DAG node bytes, not the file.
 */
export async function fetchFromBitswapRpc(
  client: PolkadotClient,
  cid: string,
): Promise<Uint8Array> {
  const hex = await client._request<string>("bitswap_v1_get", [cid]);
  return hexToBytes(hex);
}
