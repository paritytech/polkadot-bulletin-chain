// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! E2E test for the HOP Chat Attachment flow (Alice → Bob).
//!
//! Exercises the full flow described in `docs/hop.md` section 3.4.1:
//! 1. Alice generates an ephemeral ed25519 keypair
//! 2. Alice uploads a file to HOP on bulletin-westend
//! 3. Alice creates an encrypted signal (statement) on people-westend
//! 4. Bob polls the statement store, decrypts the signal
//! 5. Bob claims the file from HOP using the ephemeral key
//! 6. Verify data integrity and pool cleanup

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use codec::Encode;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sp_core::{crypto::Pair, hashing::blake2_256, Bytes};
use sp_statement_store::Statement;
use subxt_rpcs::client::{rpc_params, RpcClient};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

fn env_or(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

// ---------------------------------------------------------------------------
// HOP RPC response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PoolStatus {
    entry_count: usize,
    total_bytes: u64,
    max_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitResult {
    hash: Bytes,
    pool_status: PoolStatus,
}

// ---------------------------------------------------------------------------
// HOP RPC helpers
// ---------------------------------------------------------------------------

async fn hop_submit(
    rpc: &RpcClient,
    data: &[u8],
    recipients: Vec<Vec<u8>>,
    proof: &[u8],
) -> Result<SubmitResult, Box<dyn std::error::Error>> {
    let hex_data = Bytes(data.to_vec());
    let hex_recipients: Vec<Bytes> = recipients.into_iter().map(Bytes).collect();
    let hex_proof = Bytes(proof.to_vec());
    let result: SubmitResult = rpc
        .request("hop_submit", rpc_params![hex_data, hex_recipients, hex_proof])
        .await?;
    Ok(result)
}

async fn hop_claim(
    rpc: &RpcClient,
    hash: &[u8],
    signature: &[u8],
) -> Result<Bytes, Box<dyn std::error::Error>> {
    let data: Bytes = rpc
        .request("hop_claim", rpc_params![Bytes(hash.to_vec()), Bytes(signature.to_vec())])
        .await?;
    Ok(data)
}

async fn hop_pool_status(
    rpc: &RpcClient,
) -> Result<PoolStatus, Box<dyn std::error::Error>> {
    let status: PoolStatus = rpc.request("hop_poolStatus", rpc_params![]).await?;
    Ok(status)
}

// ---------------------------------------------------------------------------
// Statement Store RPC helpers
// ---------------------------------------------------------------------------

async fn statement_submit(
    rpc: &RpcClient,
    statement: &Statement,
) -> Result<(), Box<dyn std::error::Error>> {
    let encoded = Bytes(statement.encode());
    let _: serde_json::Value = rpc.request("statement_submit", rpc_params![encoded]).await?;
    Ok(())
}

async fn statement_broadcasts_stmt(
    rpc: &RpcClient,
    topics: Vec<[u8; 32]>,
) -> Result<Vec<Bytes>, Box<dyn std::error::Error>> {
    let result: Vec<Bytes> = rpc
        .request("statement_broadcastsStatement", rpc_params![topics])
        .await?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Crypto helpers
// ---------------------------------------------------------------------------

fn aes_encrypt(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).expect("AES encryption failed");
    // Prepend nonce to ciphertext
    let mut result = nonce_bytes.to_vec();
    result.extend_from_slice(&ciphertext);
    result
}

// ---------------------------------------------------------------------------
// Signal payload (sent via statement store)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct SignalPayload {
    cid: String,
    collator_url: String,
    ephemeral_secret_seed: String,
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let people_rpc_url = env_or("PEOPLE_RPC_URL", "ws://127.0.0.1:9944");
    let bulletin_rpc_url = env_or("BULLETIN_RPC_URL", "ws://127.0.0.1:10000");
    let file_size_kb: usize = env_or("FILE_SIZE_KB", "128").parse()?;

    println!("=== HOP E2E Chat Attachment Test ===");
    println!("People RPC:   {people_rpc_url}");
    println!("Bulletin RPC: {bulletin_rpc_url}");
    println!("File size:    {file_size_kb} KiB");
    println!();

    // Step 1: Connect RPC clients
    println!("[1/12] Connecting RPC clients...");
    let people_rpc = RpcClient::from_insecure_url(&people_rpc_url).await?;
    let bulletin_rpc = RpcClient::from_insecure_url(&bulletin_rpc_url).await?;
    println!("  Connected to both chains.");

    // Step 2: Generate shared AES-256 key (simulates prior key exchange)
    println!("[2/12] Generating shared AES-256 key...");
    let mut aes_key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut aes_key);
    println!("  AES key: 0x{}...", hex::encode(&aes_key[..8]));

    // Step 3: Generate test file data
    println!("[3/12] Generating {file_size_kb} KiB test file...");
    let mut file_data = vec![0u8; file_size_kb * 1024];
    rand::thread_rng().fill_bytes(&mut file_data);
    let file_hash = blake2_256(&file_data);
    println!("  File hash: 0x{}", hex::encode(file_hash));

    // Step 4: Alice generates ephemeral ed25519 keypair
    println!("[4/12] Alice: generating ephemeral ed25519 keypair...");
    let (ephemeral_pair, ephemeral_seed) = sp_core::ed25519::Pair::generate();
    let ephemeral_pubkey: [u8; 32] = ephemeral_pair.public().0;
    println!("  Ephemeral pubkey: 0x{}", hex::encode(ephemeral_pubkey));

    // Step 5: Alice uploads file to HOP
    println!("[5/12] Alice: uploading file to HOP...");
    let pool_before = hop_pool_status(&bulletin_rpc).await?;
    // Mock proof: any non-empty bytes work with MockPersonhoodVerifier
    let mock_proof = b"alice-personhood-proof";
    let submit_result = hop_submit(
        &bulletin_rpc,
        &file_data,
        vec![ephemeral_pubkey.to_vec()],
        mock_proof,
    )
    .await?;
    let cid = submit_result.hash.0.clone();
    println!("  CID: 0x{}", hex::encode(&cid));
    println!(
        "  Pool: {} entries, {} bytes",
        submit_result.pool_status.entry_count, submit_result.pool_status.total_bytes
    );
    assert_eq!(cid.as_slice(), &file_hash, "CID should match blake2_256 of file data");

    // Step 6: Alice creates signal payload
    println!("[6/12] Alice: creating encrypted signal payload...");
    let signal = SignalPayload {
        cid: format!("0x{}", hex::encode(&cid)),
        collator_url: bulletin_rpc_url.clone(),
        ephemeral_secret_seed: format!("0x{}", hex::encode(ephemeral_seed.as_ref())),
    };
    let signal_json = serde_json::to_vec(&signal)?;
    let encrypted_signal = aes_encrypt(&aes_key, &signal_json);
    println!("  Signal encrypted ({} bytes)", encrypted_signal.len());

    // Step 7: Alice submits statement to People Chain
    println!("[7/12] Alice: submitting statement to People Chain...");
    let topic: [u8; 32] = blake2_256(b"hop-test");
    let mut stmt = Statement::new();
    stmt.set_topic(0, topic);
    stmt.set_plain_data(encrypted_signal);
    // Sign with Alice's sr25519 dev key
    let alice_sr25519 = sp_core::sr25519::Pair::from_string("//Alice", None)
        .expect("Failed to create Alice sr25519 pair");
    stmt.sign_sr25519_private(&alice_sr25519);
    statement_submit(&people_rpc, &stmt).await?;
    println!("  Statement submitted (topic: 0x{})", hex::encode(topic));

    // Step 8: Bob polls for statements
    println!("[8/12] Bob: polling statement store...");
    let mut bob_statements = Vec::new();
    for attempt in 1..=30 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let stmts = statement_broadcasts_stmt(&people_rpc, vec![topic]).await?;
        if !stmts.is_empty() {
            bob_statements = stmts;
            println!("  Found {} statement(s) on attempt {attempt}", bob_statements.len());
            break;
        }
        if attempt % 5 == 0 {
            println!("  Attempt {attempt}/30, no statements yet...");
        }
    }
    assert!(!bob_statements.is_empty(), "Bob should have received at least one statement");

    // Step 9: Bob decrypts the statement
    println!("[9/12] Bob: decrypting statement...");
    let alice_account_id: [u8; 32] = alice_sr25519.public().0;
    let mut received_signal: Option<SignalPayload> = None;
    for stmt_bytes in &bob_statements {
        let Ok(decoded) = <Statement as codec::Decode>::decode(&mut &stmt_bytes.0[..]) else {
            continue;
        };
        if decoded.account_id() != Some(alice_account_id) {
            continue;
        }
        let Some(data) = decoded.data() else { continue };
        if data.len() <= 12 {
            continue;
        }
        let Ok(plaintext) = (|| -> Result<Vec<u8>, ()> {
            let (nonce_bytes, ciphertext) = data.split_at(12);
            let cipher = Aes256Gcm::new((&aes_key).into());
            let nonce = Nonce::from_slice(nonce_bytes);
            cipher.decrypt(nonce, ciphertext).map_err(|_| ())
        })() else {
            continue;
        };
        if let Ok(signal) = serde_json::from_slice::<SignalPayload>(&plaintext) {
            received_signal = Some(signal);
            break;
        }
    }
    let received_signal = received_signal.expect("Bob should find Alice's statement");
    println!("  CID: {}", received_signal.cid);
    println!("  Collator URL: {}", received_signal.collator_url);

    // Step 10: Bob claims file from HOP
    println!("[10/12] Bob: claiming file from HOP...");
    let seed_bytes = hex::decode(received_signal.cid.trim_start_matches("0x"))?;
    let eph_seed_hex = received_signal.ephemeral_secret_seed.trim_start_matches("0x");
    let eph_seed_bytes: [u8; 32] = hex::decode(eph_seed_hex)?
        .try_into()
        .map_err(|_| "Invalid ephemeral seed length")?;
    let bob_eph_pair = sp_core::ed25519::Pair::from_seed(&eph_seed_bytes);
    // Sign the content hash (CID) with the ephemeral key
    let claim_signature = bob_eph_pair.sign(&seed_bytes);
    let claimed_data = hop_claim(&bulletin_rpc, &seed_bytes, claim_signature.as_ref()).await?;
    println!("  Claimed {} bytes", claimed_data.0.len());

    // Step 11: Verify data integrity
    println!("[11/12] Verifying data integrity...");
    assert_eq!(
        claimed_data.0, file_data,
        "Claimed data should match original file"
    );
    println!("  Data integrity verified!");

    // Step 12: Verify pool cleanup
    println!("[12/12] Verifying pool cleanup...");
    let final_status = hop_pool_status(&bulletin_rpc).await?;
    println!(
        "  Pool: {} entries, {} bytes",
        final_status.entry_count, final_status.total_bytes
    );
    assert_eq!(
        final_status.entry_count, pool_before.entry_count,
        "Pool entry count should return to pre-submit level after claim"
    );

    println!();
    println!("SUCCESS: E2E chat attachment flow verified");

    Ok(())
}
