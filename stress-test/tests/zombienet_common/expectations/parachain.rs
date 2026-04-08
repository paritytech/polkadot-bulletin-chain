use super::Expectation;

// Parachain expected results for block capacity test.
//
// These sizes MUST match `ALL_PAYLOAD_SIZES` in `scenarios/throughput.rs`.
// To update, run the test with `--nocapture` and look at the PASS/FAIL log lines
// which print actual avg and peak values.
//
// Notes:
//   - Throughput variants are capped at **2 MiB** per store (`MAX_STORE_PAYLOAD_BYTES`).
//   - Largest variant (2MB) may still hit WASM heap / PoV limits on some nodes; expectations remain
//     optimistic for zombienet parachain CI.
pub const EXPECTATIONS: &[Expectation] = &[
	Expectation { payload_size: 1024, label: "1KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 4096, label: "4KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 32 * 1024, label: "32KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 128 * 1024, label: "128KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 512 * 1024, label: "512KB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 1024 * 1024, label: "1MB", expected: Some((1.0, 1)) },
	Expectation { payload_size: 2 * 1024 * 1024, label: "2MB", expected: Some((1.0, 1)) },
];
