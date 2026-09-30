use crate::cid_hash::{cid_of, Hashing, CODEC_DAG_PB, CODEC_RAW};

const WIRE_TYPE_VARINT: u64 = 0;
const WIRE_TYPE_64BIT: u64 = 1;
const WIRE_TYPE_LEN_DELIMITED: u64 = 2;
const WIRE_TYPE_32BIT: u64 = 5;

const PB_NODE_DATA: u32 = 1;
const PB_NODE_LINKS: u32 = 2;

const PB_LINK_HASH: u32 = 1;
const PB_LINK_NAME: u32 = 2;
const PB_LINK_TSIZE: u32 = 3;

const UNIXFS_TYPE: u32 = 1;
const UNIXFS_FILESIZE: u32 = 3;
const UNIXFS_BLOCKSIZE: u32 = 4;
const UNIXFS_TYPE_FILE: u64 = 2;

const MAX_UVARINT_BYTES: usize = 10;

pub(crate) fn put_uvarint(buf: &mut Vec<u8>, mut value: u64) {
	while value >= 0x80 {
		buf.push((value as u8) | 0x80);
		value >>= 7;
	}
	buf.push(value as u8);
}

fn put_len_field(buf: &mut Vec<u8>, field: u32, data: &[u8]) {
	put_uvarint(buf, ((field as u64) << 3) | WIRE_TYPE_LEN_DELIMITED);
	put_uvarint(buf, data.len() as u64);
	buf.extend_from_slice(data);
}

fn put_varint_field(buf: &mut Vec<u8>, field: u32, value: u64) {
	put_uvarint(buf, ((field as u64) << 3) | WIRE_TYPE_VARINT);
	put_uvarint(buf, value);
}

pub fn raw_leaf_cid(data: &[u8]) -> cid::Cid {
	cid_of(CODEC_RAW, Hashing::Sha2_256, data)
}

pub fn build_unixfs_file_root(leaves: &[(cid::Cid, usize)]) -> (Vec<u8>, cid::Cid) {
	let total_size: u64 = leaves.iter().map(|(_, size)| *size as u64).sum();

	let mut unixfs = Vec::new();
	put_varint_field(&mut unixfs, UNIXFS_TYPE, UNIXFS_TYPE_FILE);
	put_varint_field(&mut unixfs, UNIXFS_FILESIZE, total_size);
	for (_, size) in leaves {
		put_varint_field(&mut unixfs, UNIXFS_BLOCKSIZE, *size as u64);
	}

	let mut node = Vec::new();
	for (cid, size) in leaves {
		let mut link = Vec::new();
		put_len_field(&mut link, PB_LINK_HASH, &cid.to_bytes());
		put_len_field(&mut link, PB_LINK_NAME, b"");
		put_varint_field(&mut link, PB_LINK_TSIZE, *size as u64);
		put_len_field(&mut node, PB_NODE_LINKS, &link);
	}
	put_len_field(&mut node, PB_NODE_DATA, &unixfs);

	let root_cid = cid_of(CODEC_DAG_PB, Hashing::Sha2_256, &node);
	(node, root_cid)
}

pub(crate) fn read_uvarint(buf: &[u8]) -> Result<(u64, usize), String> {
	let mut value = 0u64;
	let mut shift = 0u32;
	for (index, &byte) in buf.iter().enumerate() {
		if index >= MAX_UVARINT_BYTES {
			return Err("uvarint too long".into());
		}
		value |= ((byte & 0x7f) as u64) << shift;
		if byte < 0x80 {
			return Ok((value, index + 1));
		}
		shift += 7;
	}
	Err("uvarint truncated".into())
}

fn field_len(buf: &[u8], wire_type: u64) -> Result<usize, String> {
	match wire_type {
		WIRE_TYPE_VARINT => Ok(read_uvarint(buf)?.1),
		WIRE_TYPE_64BIT => Ok(8),
		WIRE_TYPE_32BIT => Ok(4),
		WIRE_TYPE_LEN_DELIMITED => {
			let (payload_len, prefix_len) = read_uvarint(buf)?;
			Ok(prefix_len + payload_len as usize)
		},
		other => Err(format!("unsupported wire type {other}")),
	}
}

fn len_delimited_fields(buf: &[u8], wanted_field: u32) -> Result<Vec<&[u8]>, String> {
	let mut fields = Vec::new();
	let mut pos = 0;
	while pos < buf.len() {
		let (key, key_len) = read_uvarint(&buf[pos..])?;
		pos += key_len;
		let field = (key >> 3) as u32;
		let wire_type = key & 0x7;
		if field == wanted_field && wire_type == WIRE_TYPE_LEN_DELIMITED {
			let (payload_len, prefix_len) = read_uvarint(&buf[pos..])?;
			pos += prefix_len;
			let end = pos
				.checked_add(payload_len as usize)
				.filter(|&end| end <= buf.len())
				.ok_or_else(|| format!("dag-pb field {wanted_field} length exceeds buffer"))?;
			fields.push(&buf[pos..end]);
			pos = end;
		} else {
			pos += field_len(&buf[pos..], wire_type)?;
		}
	}
	Ok(fields)
}

pub fn child_links(node: &[u8]) -> Result<Vec<cid::Cid>, String> {
	let mut cids = Vec::new();
	for link in len_delimited_fields(node, PB_NODE_LINKS)? {
		if let Some(hash) = len_delimited_fields(link, PB_LINK_HASH)?.first() {
			let cid =
				cid::Cid::try_from(*hash).map_err(|error| format!("invalid child CID: {error}"))?;
			cids.push(cid);
		}
	}
	Ok(cids)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::cid_hash::cid_v1;

	const HELLO_CID_V1_RAW_SHA256: &str =
		"bafkreibm6jg3ux5qumhcn2b3flc3tyu6dmlb4xa7u5bf44yegnrjhc4yeq";

	fn test_leaves(count: u8) -> Vec<(cid::Cid, usize)> {
		(0..count).map(|index| (raw_leaf_cid(&[index; 8]), 8)).collect()
	}

	#[test]
	fn uvarint_round_trip() {
		for value in [0u64, 1, 127, 128, 300, 16_383, 16_384, u64::MAX] {
			let mut buf = Vec::new();
			put_uvarint(&mut buf, value);
			assert_eq!(read_uvarint(&buf).unwrap(), (value, buf.len()));
		}
	}

	#[test]
	fn raw_leaf_cid_is_v1_raw_sha256() {
		let cid = raw_leaf_cid(b"hello");
		assert_eq!(cid.codec(), CODEC_RAW);
		assert_eq!(cid.hash().code(), Hashing::Sha2_256.multihash_code());
		assert_eq!(cid.to_string(), HELLO_CID_V1_RAW_SHA256);
	}

	#[test]
	fn root_is_dag_pb_and_cid_matches_bytes() {
		let leaves = test_leaves(20);
		let (bytes, cid) = build_unixfs_file_root(&leaves);
		assert_eq!(cid.codec(), CODEC_DAG_PB);
		assert_eq!(cid_v1(CODEC_DAG_PB, Hashing::Sha2_256, &Hashing::Sha2_256.digest(&bytes)), cid);
		for (leaf, _) in &leaves {
			let leaf_bytes = leaf.to_bytes();
			assert!(
				bytes.windows(leaf_bytes.len()).any(|window| window == leaf_bytes.as_slice()),
				"root does not contain leaf CID {leaf}"
			);
		}
	}

	#[test]
	fn child_links_round_trips_build_unixfs_file_root() {
		let leaves = test_leaves(20);
		let (bytes, _root) = build_unixfs_file_root(&leaves);
		let decoded = child_links(&bytes).unwrap();
		let expected: Vec<cid::Cid> = leaves.iter().map(|(cid, _)| *cid).collect();
		assert_eq!(decoded, expected);
	}

	#[test]
	fn child_links_empty_for_leafless_node() {
		let (bytes, _) = build_unixfs_file_root(&[]);
		assert!(child_links(&bytes).unwrap().is_empty());
	}
}
