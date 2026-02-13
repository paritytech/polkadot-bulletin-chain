# Manifests & IPFS

A manifest is a small DAG-PB node that describes how to reassemble a large file from its chunks. It contains links (CIDs) to all chunks plus UnixFS metadata (file size, type).

## IPFS Compatibility

The SDK uses standard DAG-PB format, so:
- The root CID matches `ipfs add --chunker=size-1048576 file.bin`
- Chunks can be pinned and served on the public IPFS network
- Bulletin Chain acts as the ledger of record for these CIDs

## Manifest Structure

```protobuf
message PBNode {
  repeated PBLink Links = 2;
  optional bytes Data = 1;  // UnixFS metadata
}

message PBLink {
  optional bytes Hash = 1;   // CID of the chunk
  optional string Name = 2;
  optional uint64 Tsize = 3; // Size of the chunk
}
```

The SDKs include `DagBuilder` (Rust) / `UnixFsDagBuilder` (TS) that construct this format automatically.
