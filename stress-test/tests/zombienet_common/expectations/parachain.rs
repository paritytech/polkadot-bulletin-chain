use super::Expectation;

// Parachain expected results for block capacity test.
//
// These sizes MUST match the array in `scenarios/throughput.rs::run_block_capacity_sweep`.
// To update, run the test with `--nocapture` and look at the PASS/FAIL log lines
// which print actual avg and peak values.
//
// Notes:
//   - 2050KB is near the single-tx WASM boundary (~2MB safe on parachain)
//   - 4MB+ OOMs during block import on non-authoring nodes (WASM freeing-bump allocator 16MB heap;
//     chunking in do_store requires ~2x payload size)
//   - 8MB+ exceeds MaxTransactionSize or OOMs at validate_transaction
pub const EXPECTATIONS: &[Expectation] = &[
	Expectation { payload_size: 1024, label: "1KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 4096, label: "4KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 32 * 1024, label: "32KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 128 * 1024, label: "128KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 512 * 1024, label: "512KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 1024 * 1024, label: "1MB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 2 * 1024 * 1024, label: "2MB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 2050 * 1024, label: "2050KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 4 * 1024 * 1024, label: "4MB", expected: None },
	Expectation { payload_size: 5 * 1024 * 1024, label: "5MB", expected: None },
	Expectation { payload_size: 7 * 1024 * 1024, label: "7MB", expected: None },
	Expectation { payload_size: 7 * 1024 * 1024 + 512 * 1024, label: "7.5MB", expected: None },
	Expectation { payload_size: 8 * 1024 * 1024, label: "8MB", expected: None },
	Expectation { payload_size: 10 * 1024 * 1024, label: "10MB", expected: None },
];
