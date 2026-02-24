use crate::sc_cli::Result;
use crate::sp_inherents::{InherentData, InherentDataProvider};
use std::time::Duration;

/// Generates inherent data for the `benchmark overhead` command.
///
/// Note: Should only be used for benchmarking.
#[allow(clippy::result_large_err)]
pub fn inherent_benchmark_data() -> Result<InherentData> {
	let mut inherent_data = InherentData::new();
	let d = Duration::from_millis(0);
	let timestamp = crate::sp_timestamp::InherentDataProvider::new(d.into());

	futures::executor::block_on(timestamp.provide_inherent_data(&mut inherent_data))
		.map_err(|e| format!("creating inherent data: {e:?}"))?;
	Ok(inherent_data)
}
