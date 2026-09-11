use anyhow::{anyhow, Result};
use futures::future::{join_all, try_join_all};
use litep2p::types::multiaddr::Multiaddr;
use std::time::Duration;

use crate::{
	bitswap::{self, BitswapClient},
	cid_hash::verify_block,
	http_gateway::HttpGatewayClient,
};

pub enum ReadSource<'a> {
	Bitswap(&'a [Multiaddr]),
	HttpGateway(&'a [String]),
}

impl ReadSource<'_> {
	pub fn transport(&self) -> &'static str {
		match self {
			ReadSource::Bitswap(_) => "Bitswap",
			ReadSource::HttpGateway(_) => "HTTP Gateway",
		}
	}

	pub fn endpoints(&self) -> usize {
		match self {
			ReadSource::Bitswap(multiaddrs) => multiaddrs.len(),
			ReadSource::HttpGateway(urls) => urls.len(),
		}
	}

	pub fn max_batch_size(&self) -> usize {
		match self {
			ReadSource::Bitswap(_) => bitswap::MAX_WANTLIST_CIDS,
			ReadSource::HttpGateway(_) => usize::MAX,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchMode {
	Raw,
	Car,
}

pub enum Fetcher {
	Bitswap { client: BitswapClient, peer: litep2p::PeerId },
	Http { client: HttpGatewayClient, mode: FetchMode },
}

impl Fetcher {
	pub async fn fetch_blocks(&self, cids: &[cid::Cid], timeout: Duration) -> Result<Vec<Vec<u8>>> {
		match self {
			Fetcher::Bitswap { client, peer } => client.fetch_blocks(*peer, cids, timeout).await,
			Fetcher::Http { client, mode: FetchMode::Raw } =>
				client.fetch_blocks(cids, timeout).await,
			Fetcher::Http { client, mode: FetchMode::Car } =>
				client.fetch_dags(cids, timeout).await,
		}
	}

	pub async fn fetch_block(&self, cid: cid::Cid, timeout: Duration) -> Result<Vec<u8>> {
		let mut blocks = self.fetch_blocks(&[cid], timeout).await?;
		blocks.pop().ok_or_else(|| anyhow!("no block in response"))
	}

	pub async fn fetch_verified_block(
		&self,
		cid: cid::Cid,
		timeout: Duration,
	) -> Result<Vec<u8>, String> {
		let data = self
			.fetch_block(cid, timeout)
			.await
			.map_err(|error| format!("download failed: {error:#}"))?;
		verify_block(&cid, &data)?;
		Ok(data)
	}
}

pub async fn connect_fetchers(
	source: &ReadSource<'_>,
	concurrency: usize,
	mode: FetchMode,
) -> Result<Vec<Fetcher>> {
	let fetchers = match source {
		ReadSource::Bitswap(multiaddrs) =>
			connect_bitswap_fetchers(multiaddrs, concurrency).await?,
		ReadSource::HttpGateway(urls) => connect_http_fetchers(urls, concurrency, mode).await?,
	};
	if fetchers.is_empty() {
		anyhow::bail!("no read workers connected");
	}
	tracing::info!("{}/{concurrency} read workers connected", fetchers.len());
	Ok(fetchers)
}

async fn connect_bitswap_fetchers(
	multiaddrs: &[Multiaddr],
	concurrency: usize,
) -> Result<Vec<Fetcher>> {
	let connects = (0..concurrency).map(|worker_index| {
		let multiaddr = &multiaddrs[worker_index % multiaddrs.len()];
		async move {
			let peer = BitswapClient::peer_id_from_multiaddr(multiaddr)?;
			match bitswap::create_connected_client(multiaddr).await {
				Ok(client) => {
					tracing::info!("Worker {worker_index}: connected to peer {peer} ({multiaddr})");
					Ok(Some(Fetcher::Bitswap { client, peer }))
				},
				Err(error) => {
					tracing::warn!("Worker {worker_index}: cannot connect to {multiaddr}: {error}");
					Ok(None)
				},
			}
		}
	});
	join_all(connects).await.into_iter().filter_map(Result::transpose).collect()
}

async fn connect_http_fetchers(
	urls: &[String],
	concurrency: usize,
	mode: FetchMode,
) -> Result<Vec<Fetcher>> {
	let clients = try_join_all(urls.iter().map(|url| HttpGatewayClient::new(url))).await?;
	Ok((0..concurrency)
		.map(|worker_index| {
			let client = clients[worker_index % clients.len()].clone();
			tracing::info!("Worker {worker_index}: HTTP gateway {}", client.base_url());
			Fetcher::Http { client, mode }
		})
		.collect())
}
