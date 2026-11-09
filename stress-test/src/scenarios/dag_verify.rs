use anyhow::{anyhow, Result};
use std::{
	collections::{HashSet, VecDeque},
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc, Mutex,
	},
	time::{Duration, Instant},
};
use subxt::OnlineClient;

use crate::{
	cid_hash::CODEC_DAG_PB,
	client::BulletinConfig,
	dag_pb,
	fetch::{connect_fetchers, FetchMode, Fetcher, ReadSource},
	metrics::{metrics, LatencyKind},
	report::{compute_latency_stats, ScenarioResult},
	scenarios::bitswap_bulk_read::discover_all_items,
};

const VARIANT: &str = "bitswap-verify-dag";
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Clone, Copy)]
struct WorkItem {
	cid: cid::Cid,
	parent: Option<cid::Cid>,
}

struct Problem {
	item: WorkItem,
	reason: String,
}

#[derive(Default)]
struct Crawl {
	queue: VecDeque<WorkItem>,
	visited: HashSet<cid::Cid>,
	in_flight: usize,
}

impl Crawl {
	fn push_new(&mut self, cid: cid::Cid, parent: Option<cid::Cid>) {
		if self.visited.insert(cid) {
			self.queue.push_back(WorkItem { cid, parent });
		}
	}

	fn pop(&mut self) -> Option<WorkItem> {
		let item = self.queue.pop_front()?;
		self.in_flight += 1;
		Some(item)
	}

	fn finish(&mut self, children: &[cid::Cid], parent: cid::Cid) {
		for &child in children {
			self.push_new(child, Some(parent));
		}
		self.in_flight -= 1;
	}

	fn is_done(&self) -> bool {
		self.queue.is_empty() && self.in_flight == 0
	}
}

#[derive(Default)]
struct WorkerStats {
	dag_nodes: u64,
	bytes: u64,
	durations: Vec<Duration>,
	problems: Vec<Problem>,
}

fn describe(item: &WorkItem) -> String {
	match item.parent {
		Some(parent) => format!("{} (child of {parent})", item.cid),
		None => format!("root {}", item.cid),
	}
}

async fn fetch_and_verify(
	fetcher: &Fetcher,
	cid: cid::Cid,
) -> Result<(usize, Vec<cid::Cid>), String> {
	let data = fetcher.fetch_verified_block(cid, FETCH_TIMEOUT).await?;
	let children = if cid.codec() == CODEC_DAG_PB {
		dag_pb::child_links(&data).map_err(|error| format!("dag-pb parse failed: {error}"))?
	} else {
		Vec::new()
	};
	Ok((data.len(), children))
}

pub async fn run_dag_verify(
	client: &OnlineClient<BulletinConfig>,
	source: ReadSource<'_>,
	concurrency: usize,
	cancel: Arc<AtomicBool>,
) -> Result<ScenarioResult> {
	let roots: Vec<cid::Cid> = discover_all_items(client)
		.await?
		.into_iter()
		.filter(|item| item.cid_codec == CODEC_DAG_PB)
		.map(|item| item.cid)
		.collect();
	if roots.is_empty() {
		anyhow::bail!("No dag-pb (codec 0x70) entries found in TransactionStorage");
	}
	let transport = source.transport();
	tracing::info!(
		"DAG verify: {} dag-pb root(s), concurrency={concurrency}, transport={transport}, \
		 endpoints={}",
		roots.len(),
		source.endpoints(),
	);

	let workers = connect_fetchers(&source, concurrency, FetchMode::Raw).await?;

	let crawl = Arc::new(Mutex::new(Crawl::default()));
	{
		let mut crawl = crawl.lock().unwrap();
		for &root in &roots {
			crawl.push_new(root, None);
		}
	}

	let wall = Instant::now();
	let mut handles = Vec::with_capacity(workers.len());
	for fetcher in workers {
		let crawl = Arc::clone(&crawl);
		let cancel = Arc::clone(&cancel);
		handles.push(tokio::spawn(async move {
			let mut stats = WorkerStats::default();
			loop {
				if cancel.load(Ordering::Relaxed) {
					break;
				}
				let Some(item) = crawl.lock().unwrap().pop() else {
					if crawl.lock().unwrap().is_done() {
						break;
					}
					tokio::time::sleep(IDLE_POLL_INTERVAL).await;
					continue;
				};

				let start = Instant::now();
				let outcome = fetch_and_verify(&fetcher, item.cid).await;
				let mut children = Vec::new();
				match outcome {
					Ok((len, found_children)) => {
						let elapsed = start.elapsed();
						if item.cid.codec() == CODEC_DAG_PB {
							stats.dag_nodes += 1;
						}
						stats.bytes += len as u64;
						stats.durations.push(elapsed);
						metrics().inc_reads(VARIANT, true, 1, len as u64);
						metrics().observe_latency(VARIANT, LatencyKind::Retrieval, elapsed);
						children = found_children;
					},
					Err(reason) => {
						metrics().inc_reads(VARIANT, false, 1, 0);
						tracing::warn!("DAG verify: {}: {reason}", describe(&item));
						stats.problems.push(Problem { item, reason });
					},
				}
				crawl.lock().unwrap().finish(&children, item.cid);
			}
			stats
		}));
	}

	let mut dag_nodes = 0u64;
	let mut bytes = 0u64;
	let mut durations = Vec::new();
	let mut problems: Vec<Problem> = Vec::new();
	for handle in handles {
		let worker = handle.await.map_err(|error| anyhow!("worker task panicked: {error}"))?;
		dag_nodes += worker.dag_nodes;
		bytes += worker.bytes;
		durations.extend(worker.durations);
		problems.extend(worker.problems);
	}

	let wall_time = wall.elapsed();
	let cancelled = cancel.load(Ordering::Relaxed);
	let blocks_ok = durations.len() as u64;
	let leaves = blocks_ok - dag_nodes;
	let problem_count = problems.len() as u64;
	let all_ok = !cancelled && problem_count == 0;

	tracing::info!(
		"DAG verify: {} root(s), {dag_nodes} dag-pb node(s), {leaves} leaf block(s), \
		 {problem_count} problem(s), {} MB, wall={:.1}s, {}",
		roots.len(),
		bytes / (1024 * 1024),
		wall_time.as_secs_f64(),
		if all_ok {
			"ALL VERIFIED OK"
		} else if cancelled {
			"CANCELLED"
		} else {
			"FAILED"
		},
	);
	if problems.is_empty() {
		tracing::info!("DAG verify: no problematic CIDs");
	} else {
		tracing::error!("DAG verify: {problem_count} CID(s) with problems:");
		for problem in &problems {
			tracing::error!("  {}: {}", describe(&problem.item), problem.reason);
		}
	}

	let secs = wall_time.as_secs_f64().max(0.001);
	Ok(ScenarioResult {
		name: format!(
			"DAG-PB Verify {transport} ({} roots, {dag_nodes} nodes + {leaves} leaves, \
			 {problem_count} problems)",
			roots.len(),
		),
		variant: VARIANT.to_string(),
		duration: wall_time,
		payload_size: bytes as usize,
		retrieval_latency: compute_latency_stats(&mut durations),
		total_reads: Some(blocks_ok + problem_count),
		successful_reads: Some(blocks_ok),
		failed_reads: Some(problem_count),
		reads_per_sec: Some(blocks_ok as f64 / secs),
		read_bytes_per_sec: Some(bytes as f64 / secs),
		data_verified: Some(all_ok),
		..Default::default()
	})
}
