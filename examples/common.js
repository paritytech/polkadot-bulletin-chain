import { Keyring } from '@polkadot/keyring';
import { getPolkadotSigner } from '@polkadot-api/signer';
import { createCanvas } from "canvas";
import fs from "fs";
import assert from "assert";

export async function waitForNewBlock() {
    // TODO: wait for a new block.
    console.log('🛰 Waiting for new block...')
    return new Promise(resolve => setTimeout(resolve, 7000))
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
    const keyring = new Keyring({ type: 'sr25519' });
    const sudoAccount = keyring.addFromUri(sudoSeed);
    const whoAccount = keyring.addFromUri(accountSeed);
    
    const sudoSigner = createSigner(sudoAccount);
    const whoSigner = createSigner(whoAccount);
    
    return {
        sudoSigner,
        whoSigner,
        whoAddress: whoAccount.address
    };
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
        this.nonce = initialNonce; // BN instance from api.query.system.account
    }

    getAndIncrement() {
        const current = this.nonce;
        this.nonce = this.nonce.addn(1); // increment BN
        return current;
    }
}

/**
 * Wait for a PAPI typed API chain to be ready by checking if we can query chain state.
 * Retries until the chain is ready or max retries reached.
 */
export async function waitForChainReady(typedApi, maxRetries = 10, retryDelayMs = 2000) {

    for (let attempt = 1; attempt <= maxRetries; attempt++) {
        try {
            // Try multiple queries to ensure chain is fully synced
            const [runtimeVersion, blockNumber] = await Promise.all([
                typedApi.query.System.LastRuntimeUpgrade(),
                typedApi.query.System.Number()
            ]);
            
            const blockNum = blockNumber ?? 0;
            console.log(`✅ Chain is ready! Block #${blockNum}, Runtime: ${runtimeVersion ? 'available' : 'checking...'}`);
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
