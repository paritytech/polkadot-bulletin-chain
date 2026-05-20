pub mod parachain;

pub struct Expectation {
	pub payload_size: usize,
	pub label: &'static str,
	/// `Some((min_avg, min_peak))` = expect success with these minimums.
	/// `None` = expect 0 confirmed (rejection / WASM OOM).
	pub expected: Option<(f64, u64)>,
}
