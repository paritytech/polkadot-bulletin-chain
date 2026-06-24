// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

// Aggregate read returned by the HOP `hop_poolStatus` JSON-RPC method.
export interface HopPoolStatus {
  // Blobs currently held (live, not yet fully acked/expired).
  entryCount: number;
  // Accounted bytes: blob size + recipients * 40-byte metadata, NOT raw disk usage.
  totalBytes: number;
  // Hard capacity ceiling.
  maxBytes: number;
}

const STORAGE_KEY_HOP_INTERVAL = "bulletin-hop-refresh-secs";

export const DEFAULT_HOP_REFRESH_SECS = 10;

export function getHopRefreshSecs(): number {
  const raw = localStorage.getItem(STORAGE_KEY_HOP_INTERVAL);
  const n = raw === null ? NaN : Number(raw);
  return Number.isFinite(n) && n > 0 ? n : DEFAULT_HOP_REFRESH_SECS;
}

export function setHopRefreshSecs(secs: number): void {
  localStorage.setItem(STORAGE_KEY_HOP_INTERVAL, String(secs));
}

// HOP nodes also accept HTTPS POST JSON-RPC at any path, so we talk to them
// directly over fetch. ws/wss URLs are normalised to http/https for the POST.
function toHttpUrl(url: string): string {
  return url.replace(/^wss:\/\//i, "https://").replace(/^ws:\/\//i, "http://");
}

export async function fetchHopPoolStatus(
  url: string,
  signal?: AbortSignal,
): Promise<HopPoolStatus> {
  const res = await fetch(toHttpUrl(url), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ id: 1, jsonrpc: "2.0", method: "hop_poolStatus", params: [] }),
    signal,
  });
  if (!res.ok) {
    throw new Error(`HTTP ${res.status} ${res.statusText}`);
  }
  const json = await res.json();
  if (json.error) {
    throw new Error(json.error.message ?? "JSON-RPC error");
  }
  const result = json.result;
  if (
    !result ||
    typeof result.entryCount !== "number" ||
    typeof result.totalBytes !== "number" ||
    typeof result.maxBytes !== "number"
  ) {
    throw new Error("Unexpected hop_poolStatus response shape");
  }
  return result as HopPoolStatus;
}
