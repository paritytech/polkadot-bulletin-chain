import * as multihash from 'multiformats/hashes/digest';
import { CID } from 'multiformats/cid';
import fs from 'fs';
import { blake2AsU8a } from '@polkadot/util-crypto';
import { waitForNewBlock } from "./common";

const CHUNK_SIZE = 1 * 1024 * 1024; // 1 MB

function to_hex(input) {
  return '0x' + input.toString('hex');
}

/**
 * helper: create CID for raw data
 */
function cidFromBytes(bytes) {
  const hash = blake2AsU8a(bytes)
  // 0xb2 = the multihash algorithm family for BLAKE2b
  // 0x20 = the digest length in bytes (32 bytes = 256 bits)
  const mh = multihash.create(0xb220, hash)
  return CID.createV1(0x55, mh) // 0x55 = raw
}

export async function authorizeAccount(api, pair, who, transactions, bytes, nonceMgr) {
  const tx = api.tx.transactionStorage.authorizeAccount(who, transactions, bytes);
  const sudo_tx = api.tx.sudo.sudo(tx);
  const result = await sudo_tx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement() });
  console.log('Transaction authorizeAccount result:', result.toHuman());
}

/**
 * Read the file, chunk it, store in Bulletin and return CIDs.
 * Returns { chunks }
 */
export async function storeChunkedFile(api, pair, filePath, nonceMgr) {
  // ---- 1️⃣ Read and split a file ----
  const fileData = fs.readFileSync(filePath)
  console.log(`📁 Read ${filePath}, size ${fileData.length} bytes`)

  const chunks = []
  for (let i = 0; i < fileData.length; i += CHUNK_SIZE) {
    const chunk = fileData.subarray(i, i + CHUNK_SIZE)
    const cid = cidFromBytes(chunk)
    chunks.push({ cid, bytes: to_hex(chunk), len: chunk.length })
  }
  console.log(`✂️ Split into ${chunks.length} chunks`)

  // ---- 2️⃣ Store chunks in Bulletin (expecting just one block) ----
  for (let i = 0; i < chunks.length; i++) {
    const { cid, bytes } = chunks[i]
    console.log(`📤 Storing chunk #${i + 1} CID: ${cid}`)
    try {
      const tx = api.tx.transactionStorage.store(bytes)
      const result = await tx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement() })
      console.log(`✅ Stored chunk #${i + 1}, result:`, result.toHuman?.())
    } catch(err) {
      if (err.stack.includes("Immediately Dropped: The transaction couldn't enter the pool because of the limit")) {
        await waitForNewBlock()
        console.log("Retrying after waiting for new block")
        --i
        continue
      }
    }
  }
  return { chunks };
}

/**
 * Creates and stores metadata describing the file chunks.
 * Returns { metadataCid }
 */
export async function storeMetadata(api, pair, chunks, nonceMgr) {
  // 1️⃣ Prepare JSON metadata (without bytes)
  const metadata = {
    type: 'file',
    version: 1,
    totalChunks: chunks.length,
    totalSize: chunks.reduce((a, c) => a + c.len, 0),
    chunks: chunks.map((c, i) => ({
      index: i,
      cid: c.cid.toString(),
      length: c.len
    }))
  };

  const jsonBytes = Buffer.from(new TextEncoder().encode(JSON.stringify(metadata)));
  console.log(`🧾 Metadata size: ${jsonBytes.length} bytes`)

  // 2️⃣ Compute CID manually (same as store() function)
  const metadataCid = cidFromBytes(jsonBytes)
  console.log('🧩 Metadata CID:', metadataCid.toString())

  // 3️⃣ Store JSON bytes in Bulletin
  const tx = api.tx.transactionStorage.store(to_hex(jsonBytes));
  const result = await tx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement() })
  console.log('📤 Metadata stored in Bulletin:', result.toHuman?.())

  return { metadataCid }
}

export async function storeProof(api, sudoPair, pair, rootCID, dagFileBytes, nonceMgr, sudoNonceMgr) {
  console.log(`🧩 Storing proof for rootCID: ${rootCID.toString()} to the Bulletin`);
  // Compute CID manually (same as store() function)
  const proofCid = cidFromBytes(dagFileBytes)

  // Store DAG bytes in Bulletin
  const storeTx = api.tx.transactionStorage.store(to_hex(dagFileBytes));
  const storeResult = await storeTx.signAndSend(pair, { nonce: nonceMgr.getAndIncrement() })
  console.log('📤 DAG proof "bytes" stored in Bulletin:', storeResult.toHuman?.())

  // This can be a serious pallet, this is just a demonstration.
  const proof = `ProofCid: ${proofCid.toString()} -> rootCID: ${rootCID.toString()}`;
  const proofTx = api.tx.system.remark(proof);
  const sudoTx = api.tx.sudo.sudo(proofTx);
  const proofResult = await sudoTx.signAndSend(sudoPair, { nonce: sudoNonceMgr.getAndIncrement() });
  console.log(`📤 DAG proof - "${proof}" - stored in Bulletin:`, proofResult.toHuman?.())
  return { proofCid }
}

export async function authorizeStorage(api, sudoPair, pair, nonceMgr) {
  // Ensure enough quota.
  const auth = await api.query.transactionStorage.authorizations({ "Account": pair.address });
  console.log('Authorization info:', auth.toHuman())

  if (!auth.isSome) {
    console.log('ℹ️ No existing authorization found — requesting new one...');
  } else {
    const authValue = auth.unwrap().extent;
    const transactions = authValue.transactions.toNumber();
    const bytes = authValue.bytes.toNumber();

    if (transactions > 10 && bytes > 24 * CHUNK_SIZE) {
      console.log('✅ Account authorization is sufficient.');
      return;
    }
  }

  const transactions = 128;
  const bytes = 128 * 1024 * 1024; // 128 MB
  await authorizeAccount(api, sudoPair, pair.address, transactions, bytes, nonceMgr);
  await waitForNewBlock();
}
