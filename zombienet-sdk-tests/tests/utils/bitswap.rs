// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Bitswap client protocol implementation using litep2p for fetching data from nodes.

use super::crypto::{blake2_256, content_hash_and_cid, hash_to_cid_bytes, verify_data_matches};
use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use litep2p::{
	codec::ProtocolCodec,
	config::ConfigBuilder as Litep2pConfigBuilder,
	crypto::ed25519::Keypair,
	protocol::{Direction, TransportEvent, TransportService, UserProtocol},
	transport::websocket::config::Config as WebSocketConfig,
	types::multiaddr::Multiaddr,
	Litep2p, Litep2pEvent, PeerId, ProtocolName,
};
use prost::Message;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

mod bitswap_schema {
	include!(concat!(env!("OUT_DIR"), "/bitswap.message.rs"));
}

const BITSWAP_PROTOCOL: &str = "/ipfs/bitswap/1.2.0";

enum BitswapClientCommand {
	Fetch {
		peer: PeerId,
		cid_bytes: Vec<u8>,
		response_tx: oneshot::Sender<Result<Vec<u8>>>,
	},
}

struct PendingFetchRequest {
	peer: PeerId,
	cid_bytes: Vec<u8>,
	response_tx: oneshot::Sender<Result<Vec<u8>>>,
}

struct BitswapClientProtocol {
	cmd_rx: mpsc::Receiver<BitswapClientCommand>,
}

impl BitswapClientProtocol {
	fn new(cmd_rx: mpsc::Receiver<BitswapClientCommand>) -> Self {
		Self { cmd_rx }
	}

	fn build_wantlist(cid_bytes: &[u8]) -> Vec<u8> {
		let request = bitswap_schema::Message {
			wantlist: Some(bitswap_schema::message::Wantlist {
				entries: vec![bitswap_schema::message::wantlist::Entry {
					block: cid_bytes.to_vec(),
					priority: 1,
					cancel: false,
					want_type: bitswap_schema::message::wantlist::WantType::Block as i32,
					send_dont_have: true,
				}],
				full: false,
			}),
			blocks: vec![],
			payload: vec![],
			block_presences: vec![],
			pending_bytes: 0,
		};
		request.encode_to_vec()
	}
}

#[async_trait::async_trait]
impl UserProtocol for BitswapClientProtocol {
	fn protocol(&self) -> ProtocolName {
		ProtocolName::from(BITSWAP_PROTOCOL)
	}

	fn codec(&self) -> ProtocolCodec {
		// Bitswap uses unsigned varint length-prefixed messages
		ProtocolCodec::UnsignedVarint(Some(2 * 1024 * 1024)) // 2MB max
	}

	async fn run(mut self: Box<Self>, service: TransportService) -> litep2p::Result<()> {
		let mut service = service;
		use futures::StreamExt;

		log::debug!("Bitswap client protocol started");

		let mut connected_peers: std::collections::HashSet<PeerId> =
			std::collections::HashSet::new();
		let mut pending_connection: Vec<PendingFetchRequest> = Vec::new();
		let mut pending_outbound: std::collections::HashMap<
			litep2p::types::SubstreamId,
			(Vec<u8>, oneshot::Sender<Result<Vec<u8>>>),
		> = std::collections::HashMap::new();

		// Only one pending fetch is supported at a time. This is sufficient for
		// the current test structure where fetches are sequential.
		let mut waiting_response: Option<(PeerId, oneshot::Sender<Result<Vec<u8>>>)> = None;

		loop {
			tokio::select! {
				cmd = self.cmd_rx.recv() => {
					match cmd {
						Some(BitswapClientCommand::Fetch { peer, cid_bytes, response_tx }) => {
							log::debug!("Received fetch command for peer {:?}", peer);

							if connected_peers.contains(&peer) {
								match service.open_substream(peer) {
									Ok(substream_id) => {
										log::debug!("Opening outbound substream {:?} to peer {:?}", substream_id, peer);
										pending_outbound.insert(substream_id, (cid_bytes, response_tx));
									}
									Err(e) => {
										log::error!("Failed to open substream: {:?}", e);
										let _ = response_tx.send(Err(anyhow!("Failed to open substream: {:?}", e)));
									}
								}
							} else {
								log::debug!("Peer {:?} not connected yet, queueing fetch request", peer);
								pending_connection.push(PendingFetchRequest {
									peer,
									cid_bytes,
									response_tx,
								});
							}
						}
						None => {
							log::debug!("Command channel closed, stopping bitswap client");
							break;
						}
					}
				}

				event = service.next() => {
					match event {
						Some(TransportEvent::SubstreamOpened { peer, direction, mut substream, .. }) => {
							match direction {
								Direction::Outbound(substream_id) => {
									log::debug!("Outbound substream {:?} opened to {:?}", substream_id, peer);

									if let Some((cid_bytes, response_tx)) = pending_outbound.remove(&substream_id) {
										let wantlist = Self::build_wantlist(&cid_bytes);
										log::debug!("Sending wantlist ({} bytes) on outbound substream", wantlist.len());

										match substream.send_framed(Bytes::from(wantlist)).await {
											Ok(()) => {
												log::debug!("Wantlist sent successfully");
												waiting_response = Some((peer, response_tx));
											}
											Err(e) => {
												log::error!("Failed to send wantlist: {:?}", e);
												let _ = response_tx.send(Err(anyhow!("Failed to send wantlist: {:?}", e)));
											}
										}
									}
								}
								Direction::Inbound => {
									log::debug!("Inbound substream from {:?}", peer);

									if let Some((waiting_peer, response_tx)) = waiting_response.take() {
										if waiting_peer == peer {
											match substream.next().await {
												Some(Ok(data)) => {
													log::debug!("Received {} bytes on inbound substream", data.len());

													match bitswap_schema::Message::decode(data.as_ref()) {
														Ok(msg) => {
															log::debug!("Parsed bitswap response: {} payload blocks, {} block_presences",
																msg.payload.len(), msg.block_presences.len());

															if let Some(block) = msg.payload.first() {
																log::info!("Received block: {} bytes", block.data.len());
																let _ = response_tx.send(Ok(block.data.clone()));
															} else if !msg.block_presences.is_empty() {
																let presence = &msg.block_presences[0];
																if presence.r#type == bitswap_schema::message::BlockPresenceType::DontHave as i32 {
																	let _ = response_tx.send(Err(anyhow!("Peer does not have the block")));
																} else {
																	let _ = response_tx.send(Err(anyhow!("Peer has block but didn't send it")));
																}
															} else {
																let _ = response_tx.send(Err(anyhow!("Empty bitswap response")));
															}
														}
														Err(e) => {
															log::error!("Failed to decode bitswap message: {:?}", e);
															let _ = response_tx.send(Err(anyhow!("Failed to decode response: {:?}", e)));
														}
													}
												}
												Some(Err(e)) => {
													log::error!("Error reading from inbound substream: {:?}", e);
													let _ = response_tx.send(Err(anyhow!("Substream error: {:?}", e)));
												}
												None => {
													log::warn!("Inbound substream closed without data");
													let _ = response_tx.send(Err(anyhow!("Substream closed")));
												}
											}
										} else {
											waiting_response = Some((waiting_peer, response_tx));
											log::debug!("Unexpected inbound substream from {:?}, expected {:?}", peer, waiting_response.as_ref().map(|(p, _)| p));
										}
									} else {
										log::debug!("Unexpected inbound substream from {:?}, no pending request", peer);
									}
								}
							}
						}
						Some(TransportEvent::SubstreamOpenFailure { substream, error }) => {
							log::error!("Substream open failure {:?}: {:?}", substream, error);
							if let Some((_, response_tx)) = pending_outbound.remove(&substream) {
								let _ = response_tx.send(Err(anyhow!("Substream open failed: {:?}", error)));
							}
						}
						Some(TransportEvent::ConnectionEstablished { peer, .. }) => {
							log::debug!("Connection established with {:?}", peer);
							connected_peers.insert(peer);

							let mut remaining = Vec::new();
							for req in pending_connection.drain(..) {
								if req.peer == peer {
									log::debug!("Processing queued fetch for peer {:?}", peer);
									match service.open_substream(peer) {
										Ok(substream_id) => {
											log::debug!("Opening outbound substream {:?} to peer {:?}", substream_id, peer);
											pending_outbound.insert(substream_id, (req.cid_bytes, req.response_tx));
										}
										Err(e) => {
											log::error!("Failed to open substream: {:?}", e);
											let _ = req.response_tx.send(Err(anyhow!("Failed to open substream: {:?}", e)));
										}
									}
								} else {
									remaining.push(req);
								}
							}
							pending_connection = remaining;
						}
						Some(TransportEvent::ConnectionClosed { peer }) => {
							log::debug!("Connection closed with {:?}", peer);
							connected_peers.remove(&peer);

							if let Some((waiting_peer, response_tx)) = waiting_response.take() {
								if waiting_peer == peer {
									let _ = response_tx.send(Err(anyhow!("Connection closed")));
								} else {
									waiting_response = Some((waiting_peer, response_tx));
								}
							}

							let mut remaining = Vec::new();
							for req in pending_connection.drain(..) {
								if req.peer == peer {
									let _ = req.response_tx.send(Err(anyhow!("Connection closed before request")));
								} else {
									remaining.push(req);
								}
							}
							pending_connection = remaining;
						}
						Some(TransportEvent::DialFailure { peer, .. }) => {
							log::error!("Dial failure for {:?}", peer);
						}
						None => {
							log::debug!("Transport service closed");
							break;
						}
					}
				}
			}
		}

		Ok(())
	}
}

/// Fetch data via bitswap using litep2p's bidirectional substream pattern.
pub async fn fetch_via_bitswap(
	node_multiaddr: &str,
	data_hash: &[u8; 32],
	timeout_secs: u64,
) -> Result<Vec<u8>> {
	log::info!(
		"Fetching via bitswap from {} for hash 0x{}",
		node_multiaddr,
		hex::encode(data_hash)
	);

	let multiaddr: Multiaddr = node_multiaddr
		.parse()
		.context("Failed to parse multiaddr")?;

	let peer_id_multihash = multiaddr
		.iter()
		.find_map(|p| {
			if let litep2p::types::multiaddr::Protocol::P2p(peer_id) = p {
				Some(peer_id)
			} else {
				None
			}
		})
		.ok_or_else(|| anyhow!("Multiaddr does not contain peer ID"))?;

	let peer_id = PeerId::from_multihash(peer_id_multihash)
		.map_err(|_| anyhow!("Invalid peer ID in multiaddr"))?;

	log::debug!("Target peer ID: {:?}", peer_id);

	let (cmd_tx, cmd_rx) = mpsc::channel(8);

	let protocol = BitswapClientProtocol::new(cmd_rx);

	let config = Litep2pConfigBuilder::new()
		.with_keypair(Keypair::generate())
		.with_user_protocol(Box::new(protocol))
		.with_websocket(WebSocketConfig {
			listen_addresses: vec!["/ip4/127.0.0.1/tcp/0/ws".parse().unwrap()],
			..Default::default()
		})
		.with_keep_alive_timeout(Duration::from_secs(60))
		.build();

	let mut litep2p =
		Litep2p::new(config).map_err(|e| anyhow!("Failed to create litep2p: {:?}", e))?;

	log::debug!("Dialing {}", multiaddr);
	litep2p
		.dial_address(multiaddr.clone())
		.await
		.map_err(|e| anyhow!("Failed to dial: {:?}", e))?;

	let connected = tokio::time::timeout(
		Duration::from_secs(30),
		wait_for_connection(&mut litep2p, peer_id),
	)
	.await
	.map_err(|_| anyhow!("Timeout waiting for connection"))?;

	if !connected {
		anyhow::bail!("Failed to establish connection");
	}

	log::info!("Connected to peer, sending bitswap request");

	let cid_bytes = hash_to_cid_bytes(data_hash);

	let (response_tx, response_rx) = oneshot::channel();

	cmd_tx
		.send(BitswapClientCommand::Fetch {
			peer: peer_id,
			cid_bytes: cid_bytes.clone(),
			response_tx,
		})
		.await
		.map_err(|_| anyhow!("Failed to send command to protocol"))?;

	let result = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
		let mut response_fut = std::pin::pin!(response_rx);

		loop {
			tokio::select! {
				event = litep2p.next_event() => {
					match event {
						Some(e) => log::trace!("litep2p event: {:?}", e),
						None => {
							return Err(anyhow!("litep2p event loop ended"));
						}
					}
				}
				result = &mut response_fut => {
					match result {
						Ok(Ok(data)) => return Ok(data),
						Ok(Err(e)) => return Err(e),
						Err(_) => return Err(anyhow!("Response channel closed")),
					}
				}
			}
		}
	})
	.await
	.map_err(|_| anyhow!("Timeout waiting for bitswap response"))??;

	Ok(result)
}

async fn wait_for_connection(litep2p: &mut Litep2p, target_peer: PeerId) -> bool {
	loop {
		match litep2p.next_event().await {
			Some(Litep2pEvent::ConnectionEstablished { peer, .. }) => {
				if peer == target_peer {
					log::debug!("Connection established with target peer");
					return true;
				}
			},
			Some(Litep2pEvent::ConnectionClosed { peer, .. }) => {
				if peer == target_peer {
					log::error!("Connection closed before established");
					return false;
				}
			},
			Some(Litep2pEvent::DialFailure { address, .. }) => {
				log::error!("Dial failure for {:?}", address);
				return false;
			},
			Some(_) => continue,
			None => return false,
		}
	}
}

pub async fn verify_bitswap_fetch(
	node_multiaddr: &str,
	expected_data: &[u8],
	timeout_secs: u64,
) -> Result<bool> {
	let hash = blake2_256(expected_data);
	let (hash_hex, cid) = content_hash_and_cid(expected_data);
	log::info!(
		"Verifying bitswap fetch for content hash: {}, CID: {}",
		hash_hex,
		cid
	);

	let fetched = fetch_via_bitswap(node_multiaddr, &hash, timeout_secs).await?;
	verify_data_matches(&fetched, expected_data)
}

pub async fn verify_node_bitswap(
	node: &zombienet_sdk::NetworkNode,
	expected_data: &[u8],
	timeout_secs: u64,
	node_name: &str,
) -> Result<()> {
	log::info!("=== Verifying bitswap fetch from {} ===", node_name);
	let multiaddr = node.multiaddr();
	let result = verify_bitswap_fetch(multiaddr, expected_data, timeout_secs)
		.await
		.context(format!("Bitswap fetch from {} failed", node_name))?;
	if !result {
		anyhow::bail!("Bitswap fetch from {} returned wrong data", node_name);
	}
	log::info!("✓ Data successfully fetched from {} via bitswap", node_name);
	Ok(())
}

/// Expect DONT_HAVE - state/warp synced nodes don't have indexed transactions.
pub async fn expect_bitswap_dont_have(
	node: &zombienet_sdk::NetworkNode,
	expected_data: &[u8],
	timeout_secs: u64,
	node_name: &str,
) -> Result<()> {
	log::info!("=== Expecting bitswap DONT_HAVE from {} ===", node_name);
	let multiaddr = node.multiaddr();
	match verify_bitswap_fetch(multiaddr, expected_data, timeout_secs).await {
		Ok(_) => {
			anyhow::bail!(
				"Expected bitswap to fail with DONT_HAVE from {}, but it succeeded",
				node_name
			);
		},
		Err(e) => {
			let error_msg = e.to_string();
			if error_msg.contains("Peer does not have the block")
				|| error_msg.contains("DONT_HAVE")
			{
				log::info!(
					"✓ Bitswap correctly returned DONT_HAVE from {} (expected for synced nodes)",
					node_name
				);
				Ok(())
			} else {
				anyhow::bail!(
					"Bitswap failed with unexpected error from {}: {}. Expected 'Peer does not have the block'",
					node_name,
					error_msg
				);
			}
		},
	}
}
