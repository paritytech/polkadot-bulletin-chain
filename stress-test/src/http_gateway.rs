use anyhow::{anyhow, Result};
use bytes::Bytes;
use std::time::Duration;

use crate::dag_pb::read_uvarint;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

fn parse_car(car: &[u8]) -> Result<Vec<(cid::Cid, Vec<u8>)>> {
	let mut pos = 0usize;
	let (header_len, prefix_len) = read_uvarint(&car[pos..]).map_err(anyhow::Error::msg)?;
	pos += prefix_len + header_len as usize;
	if pos > car.len() {
		anyhow::bail!("CAR header exceeds body");
	}

	let mut blocks = Vec::new();
	while pos < car.len() {
		let (section_len, prefix_len) = read_uvarint(&car[pos..]).map_err(anyhow::Error::msg)?;
		pos += prefix_len;
		if section_len == 0 {
			break;
		}
		let end = pos + section_len as usize;
		if end > car.len() {
			anyhow::bail!("truncated CAR section");
		}
		let mut cursor = std::io::Cursor::new(&car[pos..end]);
		let cid = cid::Cid::read_bytes(&mut cursor)
			.map_err(|error| anyhow!("bad CID in CAR section: {error}"))?;
		let data = car[pos + cursor.position() as usize..end].to_vec();
		blocks.push((cid, data));
		pos = end;
	}
	Ok(blocks)
}

#[derive(Clone)]
pub struct HttpGatewayClient {
	client: reqwest::Client,
	base_url: String,
}

impl HttpGatewayClient {
	pub async fn new(base_url: &str) -> Result<Self> {
		let base_url = base_url.trim_end_matches('/').to_string();

		let url = reqwest::Url::parse(&base_url)
			.map_err(|error| anyhow!("bad URL {base_url}: {error}"))?;
		let host = url.host_str().ok_or_else(|| anyhow!("no host in URL {base_url}"))?;
		let port = url.port_or_known_default().unwrap_or(443);
		let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
			.await
			.map_err(|error| anyhow!("DNS lookup for {host} failed: {error}"))?
			.collect();
		if addrs.is_empty() {
			anyhow::bail!("DNS lookup for {host} returned no addresses");
		}

		let client = reqwest::Client::builder()
			.timeout(REQUEST_TIMEOUT)
			.resolve_to_addrs(host, &addrs)
			.build()
			.map_err(|error| anyhow!("cannot build HTTP client: {error}"))?;
		Ok(Self { client, base_url })
	}

	pub fn base_url(&self) -> &str {
		&self.base_url
	}

	async fn get(&self, url: &str, accept: &str) -> Result<Bytes> {
		let response = self
			.client
			.get(url)
			.header("Accept", accept)
			.send()
			.await
			.map_err(|error| anyhow!("GET {url}: {:#}", anyhow::Error::new(error)))?;
		let status = response.status();
		if !status.is_success() {
			anyhow::bail!("GET {url}: HTTP {status}");
		}
		response
			.bytes()
			.await
			.map_err(|error| anyhow!("GET {url}: body read: {:#}", anyhow::Error::new(error)))
	}

	async fn get_raw(&self, cid: &cid::Cid) -> Result<Vec<u8>> {
		let url = format!("{}/ipfs/{cid}?format=raw", self.base_url);
		Ok(Vec::from(self.get(&url, "application/vnd.ipld.raw").await?))
	}

	async fn get_car(&self, root: &cid::Cid) -> Result<Vec<(cid::Cid, Vec<u8>)>> {
		let url = format!("{}/ipfs/{root}?format=car&dag-scope=all", self.base_url);
		let bytes = self.get(&url, "application/vnd.ipld.car").await?;
		parse_car(&bytes).map_err(|error| anyhow!("GET {url}: bad CAR: {error}"))
	}

	pub async fn fetch_blocks(
		&self,
		cids: &[cid::Cid],
		timeout_duration: Duration,
	) -> Result<Vec<Vec<u8>>> {
		let request = futures::future::try_join_all(cids.iter().map(|cid| self.get_raw(cid)));
		tokio::time::timeout(timeout_duration, request)
			.await
			.map_err(|_| anyhow!("HTTP gateway fetch timed out"))?
	}

	pub async fn fetch_dag(
		&self,
		root: &cid::Cid,
		timeout_duration: Duration,
	) -> Result<Vec<(cid::Cid, Vec<u8>)>> {
		tokio::time::timeout(timeout_duration, self.get_car(root))
			.await
			.map_err(|_| anyhow!("HTTP gateway CAR fetch timed out"))?
	}

	pub async fn fetch_dags(
		&self,
		roots: &[cid::Cid],
		timeout_duration: Duration,
	) -> Result<Vec<Vec<u8>>> {
		let request = futures::future::try_join_all(roots.iter().map(|root| self.get_car(root)));
		let dags = tokio::time::timeout(timeout_duration, request)
			.await
			.map_err(|_| anyhow!("HTTP gateway CAR fetch timed out"))??;
		Ok(dags.into_iter().flatten().map(|(_, data)| data).collect())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::dag_pb::{put_uvarint, raw_leaf_cid};

	fn section(buf: &mut Vec<u8>, data: &[u8]) {
		let mut body = raw_leaf_cid(data).to_bytes();
		body.extend_from_slice(data);
		put_uvarint(buf, body.len() as u64);
		buf.extend_from_slice(&body);
	}

	#[test]
	fn parses_car_into_cid_data_pairs() {
		let mut car = Vec::new();
		put_uvarint(&mut car, 2);
		car.extend_from_slice(&[0xAA, 0xBB]);
		section(&mut car, b"alpha");
		section(&mut car, b"beta!!");

		let blocks = parse_car(&car).unwrap();
		assert_eq!(blocks.len(), 2);
		assert_eq!(blocks[0], (raw_leaf_cid(b"alpha"), b"alpha".to_vec()));
		assert_eq!(blocks[1], (raw_leaf_cid(b"beta!!"), b"beta!!".to_vec()));
	}
}
