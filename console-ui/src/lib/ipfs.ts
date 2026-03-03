/**
 * Default IPFS gateway URL for local Bulletin Chain node
 */
export const DEFAULT_IPFS_GATEWAY = "http://127.0.0.1:8283";

/**
 * IPFS gateway URLs for different networks
 */
export const IPFS_GATEWAYS: Record<string, string> = {
  local: "http://127.0.0.1:8283",
  paseo: "https://paseo-ipfs.polkadot.io",
  previewnet: "https://previewnet.substrate.dev",
};

/**
 * Preferred download method per network.
 * Networks with a known IPFS gateway default to "gateway",
 * others default to "p2p".
 */
export const PREFERRED_DOWNLOAD_METHOD: Record<string, "p2p" | "gateway"> = {
  local: "p2p",
  westend: "p2p",
  paseo: "gateway",
  previewnet: "gateway",
};

/**
 * Fetch content from IPFS by CID
 */
export async function fetchFromIpfs(
  cid: string,
  gateway: string = DEFAULT_IPFS_GATEWAY
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
  gateway: string = DEFAULT_IPFS_GATEWAY
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
  gateway: string = DEFAULT_IPFS_GATEWAY
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
 * Build IPFS gateway URL for a CID
 */
export function buildIpfsUrl(
  cid: string,
  gateway: string = DEFAULT_IPFS_GATEWAY
): string {
  return `${gateway}/ipfs/${cid}`;
}
