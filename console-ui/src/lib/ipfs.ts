// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

// Per-network gateway URLs live in `config/networks.ts` (`Network.ipfsGateway`).

/**
 * Fetch content from IPFS by CID
 */
export async function fetchFromIpfs(
  cid: string,
  gateway: string
): Promise<{
  data: Uint8Array;
  contentType?: string;
}> {
  const url = `${gateway}/ipfs/${cid}`;

  const response = await fetch(url);

  if (!response.ok) {
    throw new Error(`IPFS fetch failed: HTTP ${response.status} ${response.statusText}`);
  }

  const contentType = response.headers.get("content-type") || undefined;
  const arrayBuffer = await response.arrayBuffer();

  return {
    data: new Uint8Array(arrayBuffer),
    contentType,
  };
}

/**
 * Check if content exists on IPFS (HEAD request)
 */
export async function checkIpfsContent(
  cid: string,
  gateway: string
): Promise<boolean> {
  const url = `${gateway}/ipfs/${cid}`;

  try {
    const response = await fetch(url, { method: "HEAD" });
    return response.ok;
  } catch {
    return false;
  }
}

/**
 * Get content info from IPFS (size, type) without downloading full content
 */
export async function getIpfsContentInfo(
  cid: string,
  gateway: string
): Promise<{
  exists: boolean;
  size?: number;
  contentType?: string;
} | null> {
  const url = `${gateway}/ipfs/${cid}`;

  try {
    const response = await fetch(url, { method: "HEAD" });

    if (!response.ok) {
      return { exists: false };
    }

    const contentLength = response.headers.get("content-length");
    const contentType = response.headers.get("content-type");

    return {
      exists: true,
      size: contentLength ? parseInt(contentLength, 10) : undefined,
      contentType: contentType || undefined,
    };
  } catch {
    return null;
  }
}

/**
 * Fetch raw block data from IPFS gateway (using ?format=raw).
 * Returns the raw encoded block bytes, needed for parsing DAG-PB manifests.
 */
export async function fetchRawBlock(
  cid: string,
  gateway: string
): Promise<Uint8Array> {
  const url = `${gateway}/ipfs/${cid}?format=raw`;

  const response = await fetch(url, {
    headers: {
      Accept: "application/vnd.ipld.raw",
    },
  });

  if (!response.ok) {
    throw new Error(`IPFS raw block fetch failed: HTTP ${response.status} ${response.statusText}`);
  }

  const arrayBuffer = await response.arrayBuffer();
  return new Uint8Array(arrayBuffer);
}

/**
 * Build IPFS gateway URL for a CID
 */
export function buildIpfsUrl(cid: string, gateway: string): string {
  return `${gateway}/ipfs/${cid}`;
}
