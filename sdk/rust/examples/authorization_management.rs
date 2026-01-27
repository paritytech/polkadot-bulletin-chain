// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

//! Authorization management example
//!
//! This example demonstrates:
//! - Authorizing accounts to store data
//! - Authorizing specific preimages (content-addressed)
//! - Refreshing authorizations before expiry
//! - Removing expired authorizations
//! - Estimating authorization requirements
//!
//! Usage:
//!   cargo run --example authorization_management --features std

use bulletin_sdk_rust::{
	async_client::{AsyncBulletinClient, AsyncClientConfig},
	prelude::*,
	submit::TransactionSubmitter,
};
use sp_runtime::AccountId32;
use std::str::FromStr;

// Include the SubxtSubmitter from simple_store example
include!("simple_store.rs");

#[tokio::main]
async fn main() -> Result<()> {
	println!("🚀 Bulletin SDK - Authorization Management Example\n");

	// 1. Setup connection (using Alice as sudo for authorization)
	println!("📡 Connecting to Bulletin Chain...");
	let endpoint = "ws://localhost:9944";

	let sudo_signer = PairSigner::new(Pair::from_string("//Alice", None).expect("Valid dev seed"));
	let submitter = SubxtSubmitter::new(endpoint, sudo_signer).await?;
	println!("✅ Connected\n");

	let client = AsyncBulletinClient::new(submitter);

	// 2. Account Authorization Example
	println!("═══ Account Authorization Example ═══\n");

	// Define the account to authorize (Bob's account)
	let bob_account_str = "5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty"; // Bob's SS58
	let bob_account = AccountId32::from_str(bob_account_str)
		.map_err(|e| Error::InvalidConfig(format!("Invalid account: {:?}", e)))?;

	println!("👤 Target account: {}", bob_account_str);

	// Calculate authorization needed for 10 MB of data
	let data_size = 10 * 1024 * 1024; // 10 MB
	let (transactions, bytes) = client.estimate_authorization(data_size);

	println!("📊 Authorization estimate for {} MB:", data_size / 1024 / 1024);
	println!("   Transactions: {}", transactions);
	println!("   Total bytes: {} ({:.2} MB)\n", bytes, bytes as f64 / 1_048_576.0);

	// Authorize Bob's account
	println!("⏳ Authorizing account...");
	let receipt = client
		.authorize_account(bob_account.clone(), transactions, bytes)
		.await?;

	println!("✅ Account authorized!");
	println!("   Block hash: {}", receipt.block_hash);
	println!("   Tx hash: {}\n", receipt.extrinsic_hash);

	// 3. Preimage Authorization Example
	println!("═══ Preimage Authorization Example ═══\n");

	// Calculate content hash for specific data
	let data = b"Specific content to be authorized";
	let content_hash = sp_io::hashing::blake2_256(data);

	println!("📝 Content to authorize:");
	println!("   Data: {:?}", String::from_utf8_lossy(data));
	println!("   Hash: 0x{}", hex::encode(content_hash));

	// Authorize this specific preimage
	println!("\n⏳ Authorizing preimage...");
	let receipt = client
		.authorize_preimage(content_hash, data.len() as u64)
		.await?;

	println!("✅ Preimage authorized!");
	println!("   Block hash: {}", receipt.block_hash);
	println!("   Tx hash: {}\n", receipt.extrinsic_hash);

	// 4. Refresh Authorization Example
	println!("═══ Refresh Authorization Example ═══\n");

	println!("🔄 Refreshing Bob's account authorization...");
	let receipt = client
		.refresh_account_authorization(bob_account.clone())
		.await?;

	println!("✅ Authorization refreshed!");
	println!("   Block hash: {}", receipt.block_hash);
	println!("   Tx hash: {}\n", receipt.extrinsic_hash);

	println!("🔄 Refreshing preimage authorization...");
	let receipt = client
		.refresh_preimage_authorization(content_hash)
		.await?;

	println!("✅ Preimage authorization refreshed!");
	println!("   Block hash: {}", receipt.block_hash);
	println!("   Tx hash: {}\n", receipt.extrinsic_hash);

	// 5. Remove Expired Authorization Example
	println!("═══ Remove Expired Authorization Example ═══\n");
	println!("💡 Note: These will only work if authorizations have actually expired\n");

	// Create an old account that might have expired authorization
	let old_account_str = "5FLSigC9HGRKVhB9FiEo4Y3koPsNmBmLJbpXg2mp1hXcS59Y"; // Charlie
	let old_account = AccountId32::from_str(old_account_str)
		.map_err(|e| Error::InvalidConfig(format!("Invalid account: {:?}", e)))?;

	println!("⏳ Attempting to remove expired account authorization...");
	match client.remove_expired_account_authorization(old_account).await {
		Ok(receipt) => {
			println!("✅ Expired authorization removed!");
			println!("   Block hash: {}", receipt.block_hash);
			println!("   Tx hash: {}", receipt.extrinsic_hash);
		},
		Err(e) => {
			println!("ℹ️  No expired authorization found (this is normal)");
			println!("   Error: {:?}", e);
		},
	}

	println!("\n⏳ Attempting to remove expired preimage authorization...");
	match client.remove_expired_preimage_authorization(content_hash).await {
		Ok(receipt) => {
			println!("✅ Expired preimage authorization removed!");
			println!("   Block hash: {}", receipt.block_hash);
			println!("   Tx hash: {}", receipt.extrinsic_hash);
		},
		Err(e) => {
			println!("ℹ️  No expired authorization found (this is normal)");
			println!("   Error: {:?}", e);
		},
	}

	// 6. Summary
	println!("\n═══ Summary ═══\n");
	println!("✅ Demonstrated operations:");
	println!("   • Account authorization (Bob)");
	println!("   • Preimage authorization (content-addressed)");
	println!("   • Refreshing authorizations (extends expiry)");
	println!("   • Removing expired authorizations");
	println!("   • Estimating authorization requirements\n");

	println!("💡 Best Practices:");
	println!("   • Use account authorization for dynamic content");
	println!("   • Use preimage authorization when content is known ahead");
	println!("   • Refresh before expiry to maintain access");
	println!("   • Clean up expired authorizations to free storage");

	println!("\n🎉 Authorization management example completed!");

	Ok(())
}
