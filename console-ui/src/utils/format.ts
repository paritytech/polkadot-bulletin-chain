export function formatBytes(bytes: number | bigint): string {
  const b = typeof bytes === "bigint" ? Number(bytes) : bytes;
  if (b === 0) return "0 B";

  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(b) / Math.log(k));

  return `${parseFloat((b / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
}

export function formatAddress(address: string, chars = 6): string {
  if (address.length <= chars * 2 + 3) return address;
  return `${address.slice(0, chars)}...${address.slice(-chars)}`;
}

export function formatNumber(num: number | bigint): string {
  return num.toLocaleString();
}

export function formatBalance(
  balance: bigint,
  decimals: number,
  symbol: string
): string {
  const divisor = BigInt(10 ** decimals);
  const whole = balance / divisor;
  const fractional = balance % divisor;

  const fractionalStr = fractional.toString().padStart(decimals, "0").slice(0, 4);
  const trimmedFractional = fractionalStr.replace(/0+$/, "");

  if (trimmedFractional) {
    return `${whole.toLocaleString()}.${trimmedFractional} ${symbol}`;
  }
  return `${whole.toLocaleString()} ${symbol}`;
}

export function hexToBytes(hex: string): Uint8Array {
  const cleanHex = hex.startsWith("0x") ? hex.slice(2) : hex;
  const bytes = new Uint8Array(cleanHex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(cleanHex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

export function bytesToHex(bytes: Uint8Array): string {
  return "0x" + Array.from(bytes)
    .map(b => b.toString(16).padStart(2, "0"))
    .join("");
}

export function formatBlockNumber(blockNumber: number | undefined): string {
  if (blockNumber === undefined) return "-";
  return `#${blockNumber.toLocaleString()}`;
}

export function formatTimestamp(timestamp: number): string {
  return new Date(timestamp).toLocaleString();
}
