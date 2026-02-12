fn main() {
    prost_build::compile_protos(&["proto/bitswap.v1.2.0.proto"], &["proto/"]).unwrap();
}
