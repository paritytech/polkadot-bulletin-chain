// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

fn main() {
	prost_build::compile_protos(&["proto/bitswap.v1.2.0.proto"], &["proto/"]).unwrap();
}
