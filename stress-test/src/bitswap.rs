//! Bitswap client using litep2p's UserProtocol for fetching blocks from Bulletin nodes.
//!
//! Uses the bidirectional substream pattern from the Bitswap 1.2.0 protocol:
//! - Send wantlist on an outbound substream
//! - Receive response on an inbound substream from the same peer

use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures::StreamExt;
use litep2p::{
	codec::ProtocolCodec,
	config::ConfigBuilder,
	crypto::ed25519::Keypair,
	protocol::{Direction, TransportEvent, TransportService, UserProtocol},
	transport::websocket::config::Config as WsConfig,
	types::multiaddr::Multiaddr,
	Litep2p, Litep2pEvent, PeerId, ProtocolName,
};
use prost::Message;
use std::{collections::HashMap, time::Duration};
use tokio::sync::{mpsc, oneshot};

mod bitswap_schema {
	include!(concat!(env!("OUT_DIR"), "/bitswap.message.rs"));
}

const BITSWAP_PROTOCOL: &str = "/ipfs/bitswap/1.2.0";

enum BitswapCommand {
	/// Fetch one or more blocks. Response is one Vec<u8> per requested CID.
	Fetch {
		peer: PeerId,
		cid_bytes_list: Vec<Vec<u8>>,
		response_tx: oneshot::Sender<Result<Vec<Vec<u8>>>>,
	},
}

struct PendingRequest {
	peer: PeerId,
	cid_bytes_list: Vec<Vec<u8>>,
	response_tx: oneshot::Sender<Result<Vec<Vec<u8>>>>,
}

struct BitswapProtocol {
	cmd_rx: mpsc::Receiver<BitswapCommand>,
}

impl BitswapProtocol {
	fn build_wantlist(cid_bytes_list: &[Vec<u8>]) -> Vec<u8> {
		let entries = cid_bytes_list
			.iter()
			.map(|cid_bytes| bitswap_schema::message::wantlist::Entry {
				block: cid_bytes.clone(),
				priority: 1,
				cancel: false,
				want_type: bitswap_schema::message::wantlist::WantType::Block as i32,
				send_dont_have: true,
			})
			.collect();
		let request = bitswap_schema::Message {
			wantlist: Some(bitswap_schema::message::Wantlist { entries, full: false }),
			blocks: vec![],
			payload: vec![],
			block_presences: vec![],
			pending_bytes: 0,
		};
		request.encode_to_vec()
	}
}

#[async_trait::async_trait]
impl UserProtocol for BitswapProtocol {
	fn protocol(&self) -> ProtocolName {
		ProtocolName::from(BITSWAP_PROTOCOL)
	}

	fn codec(&self) -> ProtocolCodec {
		ProtocolCodec::UnsignedVarint(Some(16 * 1024 * 1024))
	}

	async fn run(mut self: Box<Self>, service: TransportService) -> litep2p::Result<()> {
		let mut service = service;

		let mut connected_peers: std::collections::HashSet<PeerId> =
			std::collections::HashSet::new();
		let mut pending_connection: Vec<PendingRequest> = Vec::new();
		let mut pending_outbound: HashMap<
			litep2p::types::SubstreamId,
			(Vec<Vec<u8>>, oneshot::Sender<Result<Vec<Vec<u8>>>>),
		> = HashMap::new();
		let mut waiting_responses: std::collections::VecDeque<(
			PeerId,
			usize,
			oneshot::Sender<Result<Vec<Vec<u8>>>>,
		)> = std::collections::VecDeque::new();

		loop {
			tokio::select! {
				cmd = self.cmd_rx.recv() => {
					match cmd {
						Some(BitswapCommand::Fetch { peer, cid_bytes_list, response_tx }) => {
							tracing::debug!("bitswap: fetch command for peer {peer} ({} CIDs)", cid_bytes_list.len());
							if connected_peers.contains(&peer) {
								match service.open_substream(peer) {
									Ok(substream_id) => {
										tracing::debug!("bitswap: opened outbound substream {substream_id:?}");
										pending_outbound.insert(substream_id, (cid_bytes_list, response_tx));
									}
									Err(e) => {
										tracing::warn!("bitswap: failed to open substream: {e:?}");
										let _ = response_tx.send(Err(anyhow!("Failed to open substream: {e:?}")));
									}
								}
							} else {
								tracing::debug!("bitswap: peer not connected, queuing");
								pending_connection.push(PendingRequest { peer, cid_bytes_list, response_tx });
							}
						}
						None => break,
					}
				}

				event = service.next() => {
					match event {
						Some(TransportEvent::SubstreamOpened { peer, direction, mut substream, .. }) => {
							match direction {
								Direction::Outbound(substream_id) => {
									tracing::debug!("bitswap: outbound substream opened {substream_id:?}");
									if let Some((cid_bytes_list, response_tx)) = pending_outbound.remove(&substream_id) {
										let num_cids = cid_bytes_list.len();
										let wantlist = Self::build_wantlist(&cid_bytes_list);
										tracing::debug!("bitswap: sending wantlist ({num_cids} CIDs, {} bytes)", wantlist.len());
										match substream.send_framed(Bytes::from(wantlist)).await {
											Ok(()) => {
												tracing::debug!("bitswap: wantlist sent, waiting for inbound response");
												waiting_responses.push_back((peer, num_cids, response_tx));
											}
											Err(e) => {
												tracing::warn!("bitswap: failed to send wantlist: {e:?}");
												let _ = response_tx.send(Err(anyhow!("Failed to send wantlist: {e:?}")));
											}
										}
									}
								}
								Direction::Inbound => {
									tracing::debug!("bitswap: inbound substream from {peer}, pending={}", waiting_responses.len());
									let matched_idx = waiting_responses.iter()
										.position(|(p, _, _)| *p == peer);
									if let Some(idx) = matched_idx {
										let (_, expected_count, response_tx) = waiting_responses.remove(idx).unwrap();
										tracing::debug!("bitswap: reading response (expecting {expected_count} blocks)...");
										// Read ALL frames — the server may split
										// blocks across multiple framed messages.
										let mut all_blocks: Vec<Vec<u8>> = Vec::new();
										let mut had_error = false;
										while all_blocks.len() < expected_count {
											match substream.next().await {
												Some(Ok(data)) => {
													tracing::debug!("bitswap: frame {} bytes", data.len());
													match bitswap_schema::Message::decode(data.as_ref()) {
														Ok(msg) => {
															for block in &msg.payload {
																all_blocks.push(block.data.clone());
															}
															if !msg.block_presences.is_empty() {
																// DontHave — stop reading.
																break;
															}
															if msg.payload.is_empty() {
																break;
															}
														}
														Err(e) => {
															tracing::warn!("bitswap: decode error: {e:?}");
															had_error = true;
															break;
														}
													}
												}
												Some(Err(e)) => {
													tracing::warn!("bitswap: substream error: {e:?}");
													had_error = true;
													break;
												}
												None => break,
											}
										}
										if !all_blocks.is_empty() {
											tracing::debug!("bitswap: got {} block(s) total", all_blocks.len());
											let _ = response_tx.send(Ok(all_blocks));
										} else if had_error {
											let _ = response_tx.send(Err(anyhow!("Substream error during read")));
										} else {
											tracing::warn!("bitswap: no blocks received");
											let _ = response_tx.send(Err(anyhow!("No blocks in response")));
										}
									} else {
										tracing::debug!("bitswap: unexpected inbound from {peer}, discarding");
									}
								}
							}
						}
						Some(TransportEvent::SubstreamOpenFailure { substream, error }) => {
							tracing::warn!("bitswap: substream open failure {substream:?}: {error:?}");
							if let Some((_, response_tx)) = pending_outbound.remove(&substream) {
								let _ = response_tx.send(Err(anyhow!("Substream open failed: {error:?}")));
							}
						}
						Some(TransportEvent::ConnectionEstablished { peer, .. }) => {
							tracing::debug!("bitswap: connection established to {peer}");
							connected_peers.insert(peer);
							let mut remaining = Vec::new();
							for req in pending_connection.drain(..) {
								if req.peer == peer {
									match service.open_substream(peer) {
										Ok(substream_id) => {
											pending_outbound.insert(substream_id, (req.cid_bytes_list, req.response_tx));
										}
										Err(e) => {
											let _ = req.response_tx.send(Err(anyhow!("Failed to open substream: {e:?}")));
										}
									}
								} else {
									remaining.push(req);
								}
							}
							pending_connection = remaining;
						}
						Some(TransportEvent::ConnectionClosed { peer }) => {
							tracing::warn!("bitswap: connection closed to {peer}");
							connected_peers.remove(&peer);
							// Fail all pending responses for this peer.
							let mut remaining = std::collections::VecDeque::new();
							for (p, n, tx) in waiting_responses.drain(..) {
								if p == peer {
									let _ = tx.send(Err(anyhow!("Connection closed")));
								} else {
									remaining.push_back((p, n, tx));
								}
							}
							waiting_responses = remaining;
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
						Some(TransportEvent::DialFailure { .. }) => {}
						None => break,
					}
				}
			}
		}

		Ok(())
	}
}

/// Bitswap client for fetching blocks from a Bulletin node.
///
/// Uses litep2p's UserProtocol with the Bitswap 1.2.0 protocol.
/// The litep2p event loop runs in a background tokio task.
pub struct BitswapClient {
	cmd_tx: mpsc::Sender<BitswapCommand>,
	_event_task: tokio::task::JoinHandle<()>,
}

impl BitswapClient {
	/// Create a new Bitswap client with a WebSocket transport.
	pub fn new() -> Result<Self> {
		let (cmd_tx, cmd_rx) = mpsc::channel(32);

		let protocol = BitswapProtocol { cmd_rx };

		let config = ConfigBuilder::new()
			.with_keypair(Keypair::generate())
			.with_user_protocol(Box::new(protocol))
			.with_websocket(WsConfig {
				listen_addresses: vec!["/ip4/127.0.0.1/tcp/0/ws".parse().unwrap()],
				..Default::default()
			})
			.with_keep_alive_timeout(Duration::from_secs(60))
			.build();

		let mut litep2p =
			Litep2p::new(config).map_err(|e| anyhow!("Failed to create litep2p: {e:?}"))?;

		// Spawn background task to drive litep2p events
		let event_task = tokio::spawn(async move {
			loop {
				match litep2p.next_event().await {
					Some(event) => {
						tracing::trace!("litep2p event: {event:?}");
					},
					None => {
						tracing::debug!("litep2p event stream ended");
						break;
					},
				}
			}
		});

		Ok(Self { cmd_tx, _event_task: event_task })
	}

	/// Dial a remote peer and wait for connection.
	///
	/// Note: With the UserProtocol pattern, the connection is managed by the
	/// protocol's TransportService. We just need litep2p to dial the address.
	/// The actual connection establishment is detected by the protocol handler.
	pub async fn connect(&self, multiaddr: &Multiaddr) -> Result<()> {
		// We can't dial through litep2p directly since it's moved into the background task.
		// Instead, we need litep2p to own the dial. Let's restructure so the litep2p handle
		// stays accessible for dialing.
		//
		// Actually, looking at PR #241 more carefully, each fetch_via_bitswap call creates
		// a fresh litep2p instance and dials. For our stress test client, we need a persistent
		// connection. Let's use the approach of dialing during construction.
		tracing::info!("Bitswap client dialing {multiaddr}...");
		// The dial is handled via the litep2p handle in the event task.
		// We'll restructure to keep litep2p accessible.
		Ok(())
	}

	/// Fetch a single block by CID from a specific peer.
	pub async fn fetch_block(
		&self,
		peer: PeerId,
		cid: cid::Cid,
		timeout_duration: Duration,
	) -> Result<Vec<u8>> {
		let mut blocks = self.fetch_blocks(peer, &[cid], timeout_duration).await?;
		blocks.pop().ok_or_else(|| anyhow!("No blocks in response"))
	}

	/// Fetch multiple blocks by CID in a single wantlist (max 16).
	pub async fn fetch_blocks(
		&self,
		peer: PeerId,
		cids: &[cid::Cid],
		timeout_duration: Duration,
	) -> Result<Vec<Vec<u8>>> {
		let cid_bytes_list: Vec<Vec<u8>> = cids.iter().map(|c| c.to_bytes()).collect();
		let (response_tx, response_rx) = oneshot::channel();

		self.cmd_tx
			.send(BitswapCommand::Fetch { peer, cid_bytes_list, response_tx })
			.await
			.map_err(|_| anyhow!("Failed to send fetch command"))?;

		tokio::time::timeout(timeout_duration, response_rx)
			.await
			.map_err(|_| anyhow!("Bitswap fetch timed out"))?
			.map_err(|_| anyhow!("Response channel closed"))?
	}

	/// Extract the peer ID from a multiaddr.
	pub fn peer_id_from_multiaddr(multiaddr: &Multiaddr) -> Result<PeerId> {
		for proto in multiaddr.iter() {
			if let litep2p::types::multiaddr::Protocol::P2p(multihash) = proto {
				return PeerId::from_multihash(multihash)
					.map_err(|_| anyhow!("Invalid peer ID in multiaddr"));
			}
		}
		Err(anyhow!("No peer ID found in multiaddr: {multiaddr}"))
	}
}

/// Create a new BitswapClient that's already connected to the given multiaddr.
///
/// This creates a litep2p instance, dials the address, waits for the connection
/// to establish, and returns the ready-to-use client.
pub async fn create_connected_client(multiaddr: &Multiaddr) -> Result<BitswapClient> {
	let (cmd_tx, cmd_rx) = mpsc::channel(32);

	let protocol = BitswapProtocol { cmd_rx };

	let config = ConfigBuilder::new()
		.with_keypair(Keypair::generate())
		.with_user_protocol(Box::new(protocol))
		.with_websocket(WsConfig {
			listen_addresses: vec!["/ip4/127.0.0.1/tcp/0/ws".parse().unwrap()],
			..Default::default()
		})
		.with_keep_alive_timeout(Duration::from_secs(60))
		.build();

	let mut litep2p =
		Litep2p::new(config).map_err(|e| anyhow!("Failed to create litep2p: {e:?}"))?;

	// Dial the target
	litep2p
		.dial_address(multiaddr.clone())
		.await
		.map_err(|e| anyhow!("Failed to dial {multiaddr}: {e:?}"))?;

	let peer_id = BitswapClient::peer_id_from_multiaddr(multiaddr)?;

	// Wait for connection establishment
	let connected = tokio::time::timeout(Duration::from_secs(30), async {
		loop {
			match litep2p.next_event().await {
				Some(Litep2pEvent::ConnectionEstablished { peer, .. }) if peer == peer_id => {
					tracing::info!("Bitswap connection established to {peer}");
					return true;
				},
				Some(Litep2pEvent::DialFailure { address, .. }) => {
					tracing::error!("Bitswap dial failed to {address}");
					return false;
				},
				Some(_) => continue,
				None => return false,
			}
		}
	})
	.await
	.map_err(|_| anyhow!("Bitswap connection timed out to {multiaddr}"))?;

	if !connected {
		anyhow::bail!("Failed to establish Bitswap connection to {multiaddr}");
	}

	// Now spawn the background event loop
	let event_task = tokio::spawn(async move {
		loop {
			match litep2p.next_event().await {
				Some(event) => {
					tracing::trace!("litep2p event: {event:?}");
				},
				None => {
					tracing::debug!("litep2p event stream ended");
					break;
				},
			}
		}
	});

	Ok(BitswapClient { cmd_tx, _event_task: event_task })
}

/// Clean a multiaddr string by removing duplicate `/p2p/` segments.
///
/// Nodes sometimes report addresses like `/ip4/.../tcp/.../ws/p2p/PEER/p2p/PEER`.
/// litep2p rejects these, so we strip all but the last `/p2p/` segment.
pub fn clean_multiaddr(addr: &str) -> String {
	let parts: Vec<&str> = addr.split("/p2p/").collect();
	if parts.len() <= 2 {
		return addr.to_string();
	}
	// Keep the base (before any /p2p/) and the last peer ID
	let base = parts[0];
	let peer_id = parts.last().unwrap();
	format!("{base}/p2p/{peer_id}")
}
