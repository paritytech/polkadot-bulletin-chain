# tx-index-tool

Inspects and repairs the indexed transaction storage of a Bulletin Chain node's database. The
tool uses two columns:

- **`TRANSACTION` (col11)** — the stored data, as `content_hash -> bytes`. Each value has a
  refcount row at key `content_hash || 0x00` containing an LE u32.
- **`BODY_INDEX` (col12)** — per block, the list of transactions its body stores or renews.

The tool reads the database files directly and does not execute the runtime. It decodes
`BODY_INDEX` with a copy of `sc-client-db`'s private `DbExtrinsic` whose `Full` variant is
opaque bytes. That copy is SCALE-identical to the original.

```bash
cargo build --release -p tx-index-tool
```

`<db>` throughout is the rocksdb directory, usually `<base-path>/chains/<chain-id>/db/full`.

## Reading a database while the node runs

RocksDB takes an exclusive lock in primary open mode, so **writes** require the node stopped.
**Reads do not.** Pass `--live` and the tool opens a secondary instance, which reads the same
files without taking the lock:

```bash
tx-index-tool list <db> --block 1889275 --live
```

A secondary instance reads the primary's state as of the last MANIFEST/WAL replay. Rows still in
the node's memtable are not visible. The tool calls `try_catch_up_with_primary` on open to
reduce this difference. Secondary mode is read-only: the tool rejects `--apply` when combined
with it. `--secondary <dir>` does the same with an explicit state directory instead of a
temporary one.

Without `--live` against a running node, the tool prints:

```
cannot open kvdb at …/db/full: IO error: While lock file: …/LOCK: Resource temporarily unavailable
note: rocksdb takes an exclusive lock — stop the node, or pass --live to attach read-only as a
      secondary instance.
```

# Scenarios

## The chain stopped producing blocks with a storage-proof error

Symptoms: the runtime panics with `Storage proof must be checked once in the block`, or the
collator logs `Missing indexed transaction 0x…`.

That panic means on-chain state records transactions stored at block `n - RetentionPeriod`, but
the block contained no `check_proof` extrinsic. Check whether the tool can build a proof from
this database:

```bash
tx-index-tool proof <db> --current --retention-period 100800 --live
```

The tool resolves the target with the same formula the node uses, then builds the proof:

```
authoring #1990075, retention 100800 → proving #1889275, randomness = hash(#1990074)

Storage proof for block #1889275
  total chunks in block:    2
  tx content hash:          0x3f7ee984…9b38
  local verification:       OK
```

| Output | Meaning |
| --- | --- |
| `local verification: OK` | the tool built a proof from this database. The cause is elsewhere: wrong target block, or the failing node has a different database |
| `no indexed body to prove at block #N` (exit 3) | the client emits no proof for that height. If on-chain state records transactions stored at that block, this mismatch causes the panic |
| `col11 missing value for hash 0x…` | the value is deleted and a block still references it. If the entry was auto-renewed, read the pruning-cadence scenario below. Otherwise read the refcount and corruption scenarios |
| `local verification: FAILED` (exit 2) | the value exists but does not match what the proof attests to |
| `chain agreement: MISMATCH` (exit 2) | with `--expect-root`: the bytes on disk differ from the bytes the chain recorded. See below |

Read the retention period from the chain. The tool reads raw columns and cannot call the runtime
API that provides it:

```bash
curl -sH 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"state_call","params":["TransactionStorageApi_retention_period","0x"]}' \
  http://127.0.0.1:9944   # SCALE u32, little-endian: 0x40890100 = 100800
```

Then check whether the target block has an indexed body. A node that warp-synced, or that was
restored from a snapshot, contains no historical indexed bodies and emits no proof for them.

```bash
tx-index-tool block <db> 1889275 --live
```

Run this on the failing node. Another node containing the data does not imply the collator
contains it.

### Comparing the proof against the chunk root recorded on chain

By default the tool derives both the proof and the root it checks against from the same bytes on
disk, so the check only confirms internal consistency. A value replaced after it was stored
still verifies against its own recomputed root. `--expect-root` compares the recomputed root
against the root the chain recorded when the data was stored:

```bash
tx-index-tool proof <db> 1889275 --expect-root 0x9b3a…c41f --live
```

```
  tx chunk root:            0x9b3a…c41f
  local verification:       OK
  expected chunk root:      0x9b3a…c41f
  chain agreement:          OK — the stored bytes are what the chain committed to
```

`MISMATCH` means the bytes on disk differ from the bytes stored for this entry. The proof is
internally consistent and the runtime would still reject it. Exit code 2, the same as a failed
local verification.

The expected root is `TransactionInfo::chunk_root` from on-chain state, stored in
`TransactionStorage::Transactions(block_number)` as a `Vec<TransactionInfo>` for the block that
stored or last renewed the data. Each element starts with `chunk_root` (32 bytes) followed by
`content_hash` (32 bytes), so the root is the 32 bytes immediately before the content hash that
`proof` prints as `tx content hash`. Read it at a block where the entry still exists:

```bash
# key = twox128("TransactionStorage") ++ twox128("Transactions")
#       ++ blake2_128(block_le) ++ block_le
curl -sH 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"state_getStorage","params":["0x<key>","0x<at>"]}' \
  http://127.0.0.1:9944
```

`trace --rpc-url` reads the same value and prints the command with the root already substituted,
so you do not have to construct the key.

## Auto-renewed data was deleted

Symptom: `Missing indexed transaction 0x…` for an entry the chain renewed on schedule.
`incident sdk-12106 drift` reports nothing for this case, because every reference and release
was symmetric.

Cause: the `--blocks-pruning` window is smaller than the interval between renewals. The node
must retain the block holding the current reference until the renewal block is imported and has
taken a new one. With a smaller window, the node releases the last reference first, deletes both
col11 rows, and the renewal cannot restore them because the renew extrinsic contains only the
hash.

```bash
tx-index-tool trace <db> 0x<hash> --live --rpc-url wss://archive-node --probe-cadence
```

`DANGLING`, with a sequence of `← reference released here` rows that the chain still has entries
for, indicates this case.

Changing the flag prevents further deletions. It does not restore deleted values. To recover,
either run `repair` per entry using a node that still contains the value, or resync with the
corrected setting. Use **full sync, not warp or fast sync**: only executing the original `store`
block writes the payload to disk.

Every auto-renewed entry is affected at its own cadence boundary, so check how many entries are
affected before repairing them individually:

```bash
tx-index-tool diff <collator-db> <archive-db> --blocks
```

## Data is missing, or a hash does not resolve

Check whether any stored value no longer hashes to the key it is filed under:

```bash
tx-index-tool list <db> --corrupted-only --live
```

```
Value entries:         112336  (32.81 GiB total)
Unexpected key rows:   0
Integrity:             112336 verified, 0 corrupted  (every value hashes to its key)
No entries matched.
```

Exit 0 means no corrupted values. For each failure the tool prints the size, the refcount, what
the bytes hash to under all three algorithms, and every block that still references the entry.
`Unexpected key rows` counts col11 rows that are neither a 32-byte value nor a `hash‖0x00`
counter. Any value above zero means the column contains a key shape the tool does not decode.

This reads every value in the column: about 48 s for 32.8 GiB. Disk reads account for most of
that time, not hashing. Restrict it with `--block N`, `--hash H` or `--min-size N` where
possible.

## Listing every block that references one entry

`trace` lists every block that references one content hash, how many references each block
contributes, and how the total compares against the counter on disk.

```bash
tx-index-tool trace <db> 0xe60057c3…b24a --live
```

```
Scan duration:         2.141129794s  (132115 BODY_INDEX entries)
Value on disk:         32768 bytes, blake2b256(value) == key
Counter on disk:       155

  block                   Δ    cum  body shape       authored               chain
  #34884                 +1      1  Indexed          2026-08-13 17:58:30 UTC
  #35885                 +1      2  Indexed          2026-08-13 19:38:42 UTC
  …
  #42892                 +1      9  MultiRenew(×1)   2026-08-14 07:25:24 UTC
  …
  #189038                +1    155  Indexed          2026-08-24 12:39:36 UTC

Alive references:      155   (sum over 155 retained reference-holding block(s))

Result: CONSISTENT — the counter holds 155, matching the ledger.
```

The tool hashes only the traced value, so this costs one `BODY_INDEX` pass and takes seconds. A
full `list` takes minutes.

col11 contains one counter per entry and no history, so the counter's value at an earlier block
is not recoverable. The list of referencing blocks is recoverable, and the tool derives the
verdict from it:

| Verdict | Meaning |
| --- | --- |
| `CONSISTENT` | the counter equals the references from retained blocks |
| `COUNTER SHORT` | references were lost. The first block to prune can reduce the counter to zero while other blocks still reference the value. See polkadot-sdk#12106 below |
| `COUNTER EXCESS` | releases were missed, so pruning will never reclaim the value |
| `DANGLING` | retained blocks reference a hash with **no value stored**. A renewal cannot repair this: the extrinsic contains only the hash, and referencing a missing counter writes nothing. Any block whose proof targets one of those blocks cannot be authored |
| `ABSENT` | no value stored and no block references it (exit 3) |

Two blocks can share a height when the node retains a fork alongside the canonical block. Each
block keeps its own `BODY_INDEX` entry and its own reference, so `trace` prints one row per
block, labels those rows with the block hash, and lists the affected heights under
`forked heights`.

### Comparing against chain state

The database records only the references it still contains. Pass `--rpc-url` and the tool reads
what the chain recorded for each block, which distinguishes a reference released by pruning from
a reference released early:

```bash
tx-index-tool trace <db> 0xe60057c3…b24a --live \
  --rpc-url wss://your-node.example --probe-cadence
```

```
Value on disk:         <absent>
Counter on disk:       <absent>
Chain:                 wss://…  head #190037, finalized #190037, RetentionPeriod 1000
Chain location:        Transactions(#189038) index 0   next proof due at #190038

  block                   Δ    cum  body shape             authored               chain
  #34884                  -      -  not in this database                          entry TransactionStorage.Stored   ← reference released here
  #35885                  -      -  not in this database                          entry DataRenewal.DataRenewed   ← reference released here
  …
  #188037                 -      -  not in this database                          entry DataRenewal.DataRenewed   ← reference released here
  #189038                +1      1  Indexed                2026-08-24 12:39:36 UTC entry DataRenewal.DataRenewed

Alive references:      1   (sum over 1 retained reference-holding block(s))

Result: DANGLING — 1 alive block(s) reference this hash and no value is stored.

The chain recorded a reference at 154 block(s) this database no longer holds:
  #34884, #35885, #36886, … (+124 more)
  Normal if those blocks were pruned. If any is inside the pruning window, its reference was
  released early.
```

`--probe-cadence` queries the renewal interval outward from the blocks already found, which
locates renewals this database no longer references. `--chain-max-blocks` limits the number of
RPC round-trips.

Two properties of the chain column:

- Auto-renewals execute in `on_initialize` and have no extrinsic, so the tool reports events:
  `TransactionStorage.Stored`, `DataRenewal.DataRenewed`, `DataRenewal.RenewalFailed`.
- The tool reads `Transactions(N)` at block `N`'s own state. A node with pruned state reports
  `state pruned` rather than `no entry`, so `no entry` is the only output that means the chain
  recorded nothing.

When the chain provides a `chunk_root`, `trace` prints a `proof --expect-root` command
containing it.

## Refcounts are wrong (polkadot-sdk#12106)

Before that fix, kvdb combined N same-key refcount operations in one transaction into a single
±1, so a counter contains one increment per referencing block instead of one per reference. The
first block to prune then decrements by its whole occurrence count, reduces the counter to zero,
and the node deletes the value while the remaining blocks still reference it.

```bash
tx-index-tool incident sdk-12106 drift <db> --live     # analyse
tx-index-tool incident sdk-12106 drift <db> --apply    # backfill, node stopped
tx-index-tool incident sdk-12106 drift <db> --live     # confirm clean (exit 0)
```

The analysis separates entries with a single referrer, where over-release saturates at zero,
from entries with more than one referring block. It prints the correct value for each counter:

```
Top 10 on-disk-drifted counters (current → correct):
  0x72a52a91…92e8  10 → 4500  (+4490)
```

`10 → 4500` across 10 referencing blocks means one increment per block where there should be one
per reference. `--apply` writes only the counter row and never modifies stored values. The tool
rejects `--apply` when combined with `--live`.

This fault occurs only with kvdb. ParityDB reference-counts col11 natively, so the same
combining cannot occur there.

## Values whose bytes were split at the wrong offset (polkadot-bulletin-chain#574)

That PR appended a `(MultiSigner, MultiSignature, u64)` tuple, which moved the boundary between
`BODY_INDEX.header` and the col11 value by 106 bytes (107 or 108 for Ecdsa). All the original
bytes remain on disk at the wrong offset, so the tool can recover them without an external copy:

```bash
tx-index-tool incident bulletin-574 realign <db> --live             # dry run, every bad entry
tx-index-tool incident bulletin-574 realign <db> --hash 0x… --live  # one entry
tx-index-tool incident bulletin-574 realign <db> --apply            # write, node stopped
```

The tool concatenates `header ++ col11_value` and searches for the split whose data side hashes
to the slot key: the known 106/107/108 sizes first, then chop-from-end, start-shift and
length-preserving window shifts within `--max-shift` (default 200). The report prints the start
shift, end chop, matching algorithm and corrected size per entry, and groups recoveries by
pattern, so a single systematic cause produces one line.

**Run `verify` before repairing.** `sc-client-db`'s `body_uncached` reassembles a body as exactly
`header ++ col11`, while the authored extrinsic was `header ++ data ++ trailing-fields`. For any
call with fields after its data, such as the pre-#574 `HopPromotion::promote`, only one of
integrity and executability can hold:

| col11 contains | hash | body reassembles to the authored bytes |
| --- | --- | --- |
| the aligned data | ✅ | ❌ block permanently unexecutable |
| the trailing window | ❌ | ✅ still replays |

`realign --apply` produces the first row and prevents the second. It also discards the trailing
fields, which exist only in the value it overwrites. No pair of values satisfies both rows, so
writing both does not help. `verify` distinguishes the two states, and identifies databases that
a previous repair already made unexecutable. Those entries print as `col11-only repair (hash ok,
NOT executable)` and `list --corrupted-only` does not report them, because their values hash
correctly.

Single renewals are stored as `Indexed { hash, header: <the whole extrinsic> }`, so their header
is complete and their value is from an earlier block. `verify` detects and skips them.

## Restoring a value from a known-good copy

```bash
tx-index-tool repair <db> 0x<hash> good-bytes.bin --live    # plan
tx-index-tool repair <db> 0x<hash> good-bytes.bin --apply   # write, node stopped
```

The tool rejects the write unless `algo(new_data) == hash`, and never modifies the counter row.
`--algo` defaults to `blake2b256`. If that is wrong, the plan prints what the on-disk value
hashes to under all three algorithms and which one, if any, reproduces the key.

For a `DANGLING` entry the counter row is also absent. Writing the value restores authoring,
because the proof path reads only the value, but with no counter pruning will never reclaim the
entry. Verify the write against the chunk root recorded on chain:

```bash
tx-index-tool proof <db> <block> --expect-root 0x<chunk_root> --live
```

`trace --rpc-url` prints that command with the root already substituted.

## Listing all entries

```bash
tx-index-tool list <db> --limit 0 --preview 0 --no-blocks --live  # everything, fastest form
tx-index-tool list <db> --sort size --desc --limit 20 --live      # the largest payloads
tx-index-tool list <db> --min-size 1000000 --live                 # only multi-MB entries
```

The header prints the entry count, total bytes and the column-wide integrity result.
`--no-blocks` skips the `BODY_INDEX` pass, which omits the created and last-seen columns and
roughly halves the runtime.

## Details for one entry

```bash
tx-index-tool list <db> --hash 0x3f7ee984…9b38 --preview 16 --live
```

```
  0x3f7ee984…9b38
    size      491 (491 B)    refcount 1    referrers 1
    integrity OK — sha2_256(value) == content hash
    created   #1889275 (2026-08-06 07:02:48 UTC)
    00000000  3a a2 65 72 6f 6f 74 73  81 d8 2a 58 25 00 01 70  |:.eroots..*X%..p|
```

This prints when the entry was stored, when it was last renewed (a second `last seen` line
appears once something renews it), how many blocks reference it, its refcount, and enough bytes
to identify the payload. In the example, `eroots` is part of a CAR header and `01 70` is a
dag-pb CID prefix. The integrity line also shows which hashing scheme the CID used:
`blake2b256` for plain `store`, `sha2_256` or `keccak256` via `store_with_cid_config`.

The tool resolves a `--hash` or `--block` filter with point lookups and skips the column walk:
about 9 s instead of 48 s on a 32.8 GiB column, and opening the secondary instance accounts for
nearly all of that.

## Inspecting one block

```bash
tx-index-tool block <db> 1889275 --live         # what the body declared
tx-index-tool list  <db> --block 1889275 --live # the state of the data it references
```

These answer different questions:

- **`block N`** reports the block: the extrinsic mix (`1 Indexed, 0 MultiRenew, 2 Full`) and the
  per-hash body shape — `Indexed`, `3×Indexed`, `MultiRenew(×4)`, `2×Indexed + MultiRenew(×3)`.
  A count above 1 for one hash in one block is the shape that polkadot-sdk#12106 mishandled.
  This command also prints the block hash when the block has no indexed body, which is how you
  obtain a parent hash for `proof --random`.
- **`list --block N`** reports the entries: integrity, algorithm, refcount, first and last
  referencing block with times, and a preview. These are chain-wide values that the block itself
  does not record.

## Retention, expiry and unreferenced values

```bash
tx-index-tool list <db> --from-block 1880000 --to-block 1890000 --live  # stored in a window
tx-index-tool list <db> --orphans-only --live                           # nothing references these
```

`--from-block` and `--to-block` bound the block where an entry was stored, which is its first
referencing block. They require the `BODY_INDEX` pass, so the tool rejects them together with
`--no-blocks`. Orphans are entries that no retained block references. They never fall inside a
range. An orphan is either a pruning candidate or an entry that remained after its references
were released.

## Comparing two nodes

When one node can author and another cannot, or one serves data that another does not, compare
their databases:

```bash
tx-index-tool diff <collator-db> <fullnode-db> --blocks --live
```

```
Best block:            A #104604   B #1472
col11 entries:         A 356   B 0   (106.61 MiB / 0 B)
Entries differing:     356
  only in A:           356
  only in B:           0
  refcount differs:    0
  size differs:        0
  integrity differs:   0

Blocks with an indexed body: A 291   B 0
  only in A (291): #54667, #54748, #55204, … (+271 more)

  0x003a3a37…1805
    only in A    357 B , refcount 1
```

Three comparisons, each detecting a different failure:

- **Entry sets** — what one node contains and the other does not. This shows whether the
  collator lost the data.
- **Refcounts per shared entry** — detects a database where the `sdk-12106` backfill was not
  applied, or where pruning diverged (`refcount     A 4500   B 10`).
- **`--blocks`** — which blocks have an indexed body on one node but not the other. Use this
  first for a stalled proof: a collator missing the body index for the proof target emits no
  proof, and this lists those blocks.

The tool compares values through their keys rather than their bytes. col11 is content-addressed,
so two entries that verify under the same key are identical, and a differing size or a one-sided
integrity failure is the only signal. Both databases are opened read-only, so `--live` works
against a running node, including comparing a live collator against a stopped one.

The body comparison keys on `(block number, block hash)`, so a retained fork and the canonical
block at one height are counted separately. `bodies_a` and `bodies_b` therefore count indexed
bodies, not heights, while `only_in_a`, `only_in_b` and `refs_differ` list heights.

Exit 0 means the databases are identical, 2 means they differ. `--limit N` caps the per-entry
lines (0 prints all). The summary counts always describe the whole comparison, not only the
printed rows.

# Command reference

| Command | What it does |
| --- | --- |
| `list <db>` | Stored data: size, hash algorithm, refcount, creating block and time, a `hexdump -C` preview, and an integrity check per entry |
| `block <db> <n>` | Which stored transactions one block's body references, with their on-disk state |
| `proof <db> <n> [--random <hex>] [--expect-root <hex>]` | Recomputes the storage proof the inherent provider emits and verifies it. `--expect-root` also compares against the root the chain recorded |
| `proof <db> --current`&nbsp;/&nbsp;`--authoring N` `--retention-period R` | The same, resolving target and randomness with the formulas the node uses |
| `trace <db> <hash> [--rpc-url URL] [--probe-cadence]` | Lists every block referencing one entry, its contribution, and the verdict on the counter. With a URL, compares each block against chain state and events |
| `diff <db> <other> [--blocks] [--limit N]` | Compares two databases: entries only one contains, refcount, size and integrity differences, and optionally which blocks have an indexed body |
| `repair <db> <hash> <file> [--algo N] [--apply]` | Overwrites one corrupted value with known-good bytes |
| `incident sdk-12106 drift <db> [--apply]` | Counters left short by the kvdb refcount combining. `--apply` sets each to its true reference count |
| `incident bulletin-574 verify <db>` | Classifies every indexed entry's `BODY_INDEX.header` and col11 split: healthy, single renewal, original mis-split, or a col11-only repair that made the block unexecutable |
| `incident bulletin-574 realign <db> [--hash H] [--max-shift N] [--apply]` | Recovers values whose `BODY_INDEX.header ++ col11` split moved |

Faults from one specific past incident are grouped under `incident` and named for the pull
request that caused or fixed them. Each diagnoses one bug. The other commands answer general
questions about the database.

`list` filters: `--limit N` (0 = all), `--preview N` bytes, `--sort block|size|refcount|hash`,
`--desc`, `--corrupted-only`, `--orphans-only`, `--min-size N`, `--hash <hex>`, `--block N`,
`--from-block N`, `--to-block N`, `--no-blocks`.

Every command accepts `--live` or `--secondary <dir>`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | No findings, or the query found the requested entry |
| 2 | Findings: drifted counters, a corrupted entry, a failed proof, an unrecoverable entry |
| 3 | A specific block or hash was requested and does not exist |
| 64 | Usage error |
| 74 | I/O failure, including a database that cannot be opened |

Exit 3 applies only to targeted lookups. A filter that matches nothing, such as
`--corrupted-only` on a database with no corrupted values, exits 0, so that command works as a
monitoring check.

## Limits

- **Writes require the node stopped.** The tool rejects `--apply` with `--live` or `--secondary`.
  Run the repair on a copy of the database first, then run the matching read-only command to
  check the result.
- **You must supply the retention period.** The tool reads raw columns and cannot call the
  runtime API, so `proof --current` requires `--retention-period`. See the RPC call above.
- **Proof verification is local by default.** Without `--expect-root` it checks only that a
  proof built from this database is internally consistent, not that the bytes match what the
  chain recorded.
- **A renewal against data the node does not contain succeeds on chain and writes nothing to
  col11.** `store_or_reference` calls `tx.reference(...)`, which writes nothing when the counter
  is absent, while `BODY_INDEX` records the reference. `sp_io::transaction_index::renew` returns
  `()`, so the runtime cannot detect this. `trace` reports the result as `DANGLING`.
- **A `DANGLING` entry is not repaired by later renewals.** col11 is written only by a `store`
  extrinsic containing the payload inline, or by `renew_payloads` supplied through
  `BlockImportParams` at import. A renew extrinsic contains only the hash.
- **RocksDB only** for now. See below.
- **File descriptors:** RocksDB requires thousands, and an unlimited count in `--live` mode. The
  tool raises its own soft limit at startup. If that fails it prints a warning, and
  `ulimit -n 65536` is the workaround.
- **Schema version:** the column indices are part of the on-disk format, so the tool compares the
  `db_version` file next to the data against the version it was written for (4) and warns on a
  mismatch.
- **Timestamps are heuristic.** They come from each block's `Timestamp::set` inherent, accepted
  as a bare extrinsic whose call is one range-checked `Compact<u64>` and nothing else. No call
  indices are hardcoded. The reports label these values as heuristic.

## ParityDB

Not supported yet. When it is supported:

- **Works:** `block`, `proof`, `list --block/--hash`, which are all point lookups. A full `list`
  is also possible: `iter_column_while` yields each value with its native refcount, and because
  col11 is content-addressed the tool can recompute the hash by hashing the value, which is the
  integrity check.
- **Degraded:** the tool cannot determine a corrupted entry's key. Hashing its value produces no
  match, and the current format siphashes the stored index key above byte 16. `BODY_INDEX` keys
  are one-way hashed too, so created and last-seen times and orphan detection would require a
  walk over block numbers through `KEY_LOOKUP` instead of over database keys.
- **Rejected:** everything that writes col11, and both `incident` commands. On a `ref_counted`
  column, `Set` on an existing key only increments the refcount and discards the supplied value
  (`// Replace is not supported`), so a repair would change nothing while incrementing the count.
  `sdk-12106` does not apply there, because ParityDB counts references itself and has no counter
  row to combine.

ParityDB does not verify that a value hashes to its key. `preimage: true` declares the property
and ParityDB does not check it, so the same corruption is possible there, and re-storing the
correct bytes does not repair it.
