use sha2::Digest as _;

pub const CODEC_RAW: u64 = 0x55;
pub const CODEC_DAG_PB: u64 = 0x70;

const MULTIHASH_BLAKE2B_256: u64 = 0xb220;
const MULTIHASH_SHA2_256: u64 = 0x12;
const MULTIHASH_KECCAK_256: u64 = 0x1b;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hashing {
	Blake2b256,
	Sha2_256,
	Keccak256,
}

impl Hashing {
	pub fn from_variant_index(index: u8) -> Option<Self> {
		match index {
			0 => Some(Self::Blake2b256),
			1 => Some(Self::Sha2_256),
			2 => Some(Self::Keccak256),
			_ => None,
		}
	}

	pub fn from_multihash_code(code: u64) -> Option<Self> {
		match code {
			MULTIHASH_BLAKE2B_256 => Some(Self::Blake2b256),
			MULTIHASH_SHA2_256 => Some(Self::Sha2_256),
			MULTIHASH_KECCAK_256 => Some(Self::Keccak256),
			_ => None,
		}
	}

	pub fn multihash_code(self) -> u64 {
		match self {
			Self::Blake2b256 => MULTIHASH_BLAKE2B_256,
			Self::Sha2_256 => MULTIHASH_SHA2_256,
			Self::Keccak256 => MULTIHASH_KECCAK_256,
		}
	}

	pub fn runtime_variant_name(self) -> &'static str {
		match self {
			Self::Blake2b256 => "Blake2b256",
			Self::Sha2_256 => "Sha2_256",
			Self::Keccak256 => "Keccak256",
		}
	}

	pub fn digest(self, data: &[u8]) -> [u8; 32] {
		match self {
			Self::Blake2b256 => crate::client::blake2b_256(data),
			Self::Sha2_256 => sha2::Sha256::digest(data).into(),
			Self::Keccak256 => sha3::Keccak256::digest(data).into(),
		}
	}
}

pub fn cid_v1(codec: u64, hashing: Hashing, digest: &[u8; 32]) -> cid::Cid {
	let multihash = cid::multihash::Multihash::<64>::wrap(hashing.multihash_code(), digest)
		.expect("32-byte digest always wraps");
	cid::Cid::new_v1(codec, multihash)
}

pub fn cid_of(codec: u64, hashing: Hashing, data: &[u8]) -> cid::Cid {
	cid_v1(codec, hashing, &hashing.digest(data))
}

pub fn verify_block(cid: &cid::Cid, data: &[u8]) -> Result<(), String> {
	let multihash = cid.hash();
	let hashing = Hashing::from_multihash_code(multihash.code())
		.ok_or_else(|| format!("unsupported multihash code 0x{:x}", multihash.code()))?;
	if hashing.digest(data) == multihash.digest() {
		Ok(())
	} else {
		Err(format!("hash mismatch ({} bytes downloaded)", data.len()))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn verifies_matching_blocks_and_rejects_tampered() {
		let data = b"the quick brown fox";
		for hashing in [Hashing::Blake2b256, Hashing::Sha2_256, Hashing::Keccak256] {
			let cid = cid_of(CODEC_RAW, hashing, data);
			assert!(verify_block(&cid, data).is_ok(), "{hashing:?}");
			assert!(verify_block(&cid, b"tampered").is_err(), "{hashing:?}");
		}
	}

	#[test]
	fn rejects_unsupported_multihash() {
		let sha1_code = 0x11;
		let multihash = cid::multihash::Multihash::<64>::wrap(sha1_code, &[0u8; 20]).unwrap();
		let cid = cid::Cid::new_v1(CODEC_RAW, multihash);
		let error = verify_block(&cid, b"anything").unwrap_err();
		assert!(error.contains("unsupported"), "got: {error}");
	}

	#[test]
	fn variant_index_and_multihash_code_round_trip() {
		for index in 0..3u8 {
			let hashing = Hashing::from_variant_index(index).unwrap();
			assert_eq!(Hashing::from_multihash_code(hashing.multihash_code()), Some(hashing));
		}
		assert_eq!(Hashing::from_variant_index(3), None);
	}
}
