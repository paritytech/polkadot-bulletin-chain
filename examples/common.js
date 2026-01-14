import { Keyring } from '@polkadot/keyring';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { createCanvas } from "canvas";
import fs from "fs";
import assert from "assert";

// ---- CONFIG ----
export const WS_ENDPOINT = 'ws://127.0.0.1:10000'; // Bulletin node
export const IPFS_API = 'http://127.0.0.1:5001';   // Local IPFS daemon
export const HTTP_IPFS_API = 'http://127.0.0.1:8080';   // Local IPFS HTTP gateway
export const CHUNK_SIZE = 1 * 1024 * 1024; // 1 MiB
// -----------------

// TODO: replace with PAPI
export async function waitForNewBlock() {
  // TODO: wait for a new block.
  console.log('🛰 Waiting for new block...')
  return new Promise(resolve => setTimeout(resolve, 8000))
}

/**
 * Creates a PAPI-compatible signer from a Keyring account
 */
export function createSigner(account) {
  return getPolkadotSigner(
    account.publicKey,
    'Sr25519',
    (input) => account.sign(input)
  );
}

export function setupKeyringAndSigners(sudoSeed, accountSeed) {
  const { signer: sudoSigner, _ } = newSigner(sudoSeed);
  const { signer: whoSigner, address: whoAddress } = newSigner(accountSeed);

  return {
    sudoSigner,
    whoSigner,
    whoAddress
  };
}

export function newSigner(seed) {
  const keyring = new Keyring({ type: 'sr25519' });
  const account = keyring.addFromUri(seed);
  const signer = createSigner(account);
  return {
    signer,
    address: account.address
  }
}

/**
 * Generates (dynamic) images based on the input text.
 */
export function generateTextImage(file, text, width = 800, height = 600) {
  const canvas = createCanvas(width, height);
  const ctx = canvas.getContext("2d");

  // 🎨 Background
  ctx.fillStyle = randomColor();
  ctx.fillRect(0, 0, width, height);

  // 🟠 Random shapes
  for (let i = 0; i < 15; i++) {
    ctx.beginPath();
    ctx.fillStyle = randomColor();
    ctx.arc(
      Math.random() * width,
      Math.random() * height,
      Math.random() * 120,
      0,
      Math.PI * 2
    );
    ctx.fill();
  }

  // ✍️ Draw your text
  ctx.font = "bold 40px Sans";
  ctx.fillStyle = "white";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";

  // Add text with shadow for readability
  ctx.shadowColor = "black";
  ctx.shadowBlur = 8;

  ctx.fillText(text, width / 2, height / 2);

  let jpegBytes = canvas.toBuffer("image/jpeg");
  fs.writeFileSync(file, jpegBytes);
  console.log("Saved to file:", file);
}

function randomColor() {
  return `rgb(${rand255()}, ${rand255()}, ${rand255()})`;
}

function rand255() {
  return Math.floor(Math.random() * 256);
}

export function filesAreEqual(path1, path2) {
  const data1 = fs.readFileSync(path1);
  const data2 = fs.readFileSync(path2);
  assert.deepStrictEqual(data1.length, data2.length)

  for (let i = 0; i < data1.length; i++) {
    assert.deepStrictEqual(data1[i], data2[i])
  }
}

export async function fileToDisk(outputPath, fullBuffer) {
  await new Promise((resolve, reject) => {
    const ws = fs.createWriteStream(outputPath);
    ws.write(fullBuffer);
    ws.end();
    ws.on('finish', resolve);
    ws.on('error', reject);
  });
  console.log(`💾 File saved to: ${outputPath}`);
}

export class NonceManager {
  constructor(initialNonce) {
    this.nonce = BigInt(initialNonce);
  }

  getAndIncrement() {
    const current = this.nonce;
    this.nonce += 1n;
    return current;
  }
}
