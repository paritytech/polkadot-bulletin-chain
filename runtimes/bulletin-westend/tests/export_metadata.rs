//! Temporary helper: export runtime metadata for sdk/metadata.scale.

#[test]
fn export_metadata() {
	let Ok(path) = std::env::var("EXPORT_METADATA_PATH") else { return };
	sp_io::TestExternalities::default().execute_with(|| {
		let metadata =
			bulletin_westend_runtime::Runtime::metadata_at_version(16).expect("v16 supported");
		std::fs::write(&path, &*metadata).expect("write metadata");
	});
}
