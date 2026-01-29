import { Keyring } from '@polkadot/keyring';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { blake2AsU8a, keccak256AsU8a, sha256AsU8a } from '@polkadot/util-crypto'
import { createCanvas } from "canvas";
import fs from "fs";
import assert from "assert";

// ---- CONFIG ----
export const DEFAULT_IPFS_API_URL = 'http://127.0.0.1:5011';     // IPFS HTTP API (for ipfs-http-client)
export const DEFAULT_IPFS_GATEWAY_URL = 'http://127.0.0.1:8283'; // IPFS HTTP Gateway (for /ipfs/CID requests)
export const CHUNK_SIZE = 1 * 1024 * 1024; // 1 MiB
// -----------------

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
 * Generates images with predefined file size targets.
 *
 * @param {string} file
 * @param {string} text
 * @param {"small" | "big32" | "big64" | "big96"} size
 */
export function generateTextImage(file, text, size = "small") {
    console.log(`Generating ${size} image with text: ${text} to the file: ${file}...`);
    const presets = {
        small: {
            width: 200,
            height: 100,
            quality: 0.6,          // few KB
            shapes: 100,
            noise: 1,
        },
        // ~33 MiB
        big32: {
            width: 6500,
            height: 5500,
            quality: 0.95,
            shapes: 1000,
            noise: 50,
            targetBytes: 32 * 1024 * 1024,
        },
        // ~64 MiB
        big64: {
            width: 7500,
            height: 5500,
            quality: 0.95,
            shapes: 1000,
            noise: 50,
            targetBytes: 65 * 1024 * 1024,
        },
        // ~96 MiB
        big96: {
            width: 9000,
            height: 6500,
            quality: 0.95,
            shapes: 1000,
            noise: 50,
            targetBytes: 98 * 1024 * 1024,
        },
    };

    const cfg = presets[size];
    if (!cfg) {
        throw new Error(`Unknown size preset: ${size}`);
    }

    const canvas = createCanvas(cfg.width, cfg.height);
    const ctx = canvas.getContext("2d");

    // 🎨 Background
    ctx.fillStyle = randomColor();
    ctx.fillRect(0, 0, cfg.width, cfg.height);

    // 🟠 Random shapes (adds entropy)
    for (let i = 0; i < cfg.shapes; i++) {
        ctx.beginPath();
        ctx.fillStyle = randomColor();
        ctx.arc(
            Math.random() * cfg.width,
            Math.random() * cfg.height,
            Math.random() * (cfg.width / 10),
            0,
            Math.PI * 2
        );
        ctx.fill();
    }

    // ✍️ Text
    ctx.font = `bold ${Math.floor(cfg.width / 20)}px Sans`;
    ctx.fillStyle = randomColor();
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.shadowColor = randomColor();
    ctx.shadowBlur = 10;
    ctx.fillText(text, cfg.width / 2, cfg.height / 2);
    addNoise(ctx, cfg.width, cfg.height, cfg.noise);

    // 🔧 Big images: tune quality to hit target size
    let imageBytes;
    if ((size === "big32" || size === "big64" || size === "big96") && cfg.targetBytes) {
        let quality = cfg.quality;

        do {
            imageBytes = canvas.toBuffer("image/jpeg", {
                quality,
                chromaSubsampling: false,
            });
            quality -= 0.02;
        } while (
            imageBytes.length > cfg.targetBytes &&
            quality > 0.6
        );
    } else {
        // Small images: single pass
        imageBytes = canvas.toBuffer("image/jpeg", {
            quality: cfg.quality,
            chromaSubsampling: false,
        });
    }

    fs.writeFileSync(file, imageBytes);
    console.log(
        `Saved ${size} image:`,
        (imageBytes.length / 1024 / 1024).toFixed(2),
        "MiB"
    );
}

function addNoise(ctx, width, height) {
    const img = ctx.getImageData(0, 0, width, height);
    const data = img.data;

    for (let i = 0; i < data.length; i += 4) {
        data[i]     = rand255(); // R
        data[i + 1] = rand255(); // G
        data[i + 2] = rand255(); // B
    }

    ctx.putImageData(img, 0, 0);
}

function randomColor() {
  return `rgb(${rand255()}, ${rand255()}, ${rand255()})`;
}

function rand(intensity) {
    return (Math.random() * intensity - intensity / 2) | 0;
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

/**
 * Wait for a PAPI typed API chain to be ready by checking runtime constants.
 * Retries until the chain is ready or max retries reached.
 */
export async function waitForChainReady(typedApi, maxRetries = 10, retryDelayMs = 2000) {

    for (let attempt = 1; attempt <= maxRetries; attempt++) {
        try {
            // Check runtime constants to verify chain is accessible
            const version = typedApi.constants.System.Version;
            console.log(`✅ Chain is ready! Runtime: ${version.spec_name} v${version.spec_version}`);
            return true;
        } catch (error) {
            if (attempt < maxRetries) {
                console.log(`⏳ Chain not ready yet (attempt ${attempt}/${maxRetries}), retrying in ${retryDelayMs/1000}s... Error: ${error.message}`);
                await new Promise(resolve => setTimeout(resolve, retryDelayMs));
            } else {
                console.log(`⚠️ Chain readiness check failed after ${maxRetries} attempts. Proceeding anyway... Error: ${error.message}`);
                return false;
            }
        }
    }
    return false;
}

export function getContentHash(bytes, mhCode = 0xb220) {
  switch (mhCode) {
    case 0xb220: // blake2b-256
      return blake2AsU8a(bytes);
    case 0x12:   // sha2-256
      return sha256AsU8a(bytes);
    case 0x1b:   // keccak-256
      return keccak256AsU8a(bytes);
    default:
      throw new Error("Unhandled multihash code: " + mhCode);
  }
}

// Convert multihash code to HashingAlgorithm enum for the runtime
export function toHashingEnum(mhCode) {
  switch (mhCode) {
    case 0xb220: return { type: "Blake2b256" };
    case 0x12:   return { type: "Sha2_256" };
    case 0x1b:   return { type: "Keccak256" };
    default:     throw new Error(`Unhandled multihash code: ${mhCode}`);
  }
}

export function toHex(bytes) {
  return '0x' + Buffer.from(bytes).toString('hex');
}

// // Try uncoment and: node common.js generateTextImage "B4" big
//
// const [, , command, ...args] = process.argv;
//
// switch (command) {
//     case "generateTextImage": {
//         const [text, size = "small"] = args;
//         generateTextImage(text + "-" + size + "output.jpeg", text, size);
//         break;
//     }
//
//     default:
//         console.error("Unknown command:", command);
//         console.error("Usage:");
//         console.error(
//             '  node common.js generateTextImage "TEXT" [small|big32|big64|big96]'
//         );
//         process.exit(1);
// }
