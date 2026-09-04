# tx-index-tool

Inspects and repairs the indexed transaction storage of a Bulletin Chain node's database. Two
columns matter:

- **`TRANSACTION` (col11)** — the stored data, as `content_hash -> bytes`, with a
  `content_hash || 0x00 -> LE u32` refcount row beside each value.
- **`BODY_INDEX` (col12)** — per block, the list of transactions its body stores or renews.

It works directly on the database files without a runtime: `BODY_INDEX` is decoded through a
mirror of `sc-client-db`'s private `DbExtrinsic` whose `Full` variant is opaque bytes, which is
SCALE-identical to it.

```bash
cargo build --release -p tx-index-tool
```

`<db>` throughout is the rocksdb directory, typically `<base-path>/chains/<chain-id>/db/full`.

## Reading a database while the node is running

RocksDB's exclusive lock belongs to the *primary* open mode, so **writes** need the node
stopped. **Reads don't** — add `--live` and the tool attaches as a secondary instance, reading
the same files without touching the lock:

```bash
tx-index-tool list <db> --block 1889275 --live
```

A secondary sees the primary's state as of the last MANIFEST/WAL replay, so rows still in the
node's memtable are not visible; the tool calls `try_catch_up_with_primary` on open to narrow
that gap. It is read-only — `--apply` is refused in combination with it. `--secondary
<dir>` is the same thing with an explicit state directory instead of a temporary one.

Without `--live` against a running node you get the lock error, and a pointer:

```
cannot open kvdb at …/db/full: IO error: While lock file: …/LOCK: Resource temporarily unavailable
note: rocksdb takes an exclusive lock — stop the node, or pass --live to attach read-only as a
      secondary instance.
```

# Scenarios

## The chain stopped producing blocks with a storage-proof error

Symptoms: the runtime panics with `Storage proof must be checked once in the block`, or the
collator logs `Missing indexed transaction 0x…`.

That assertion means on-chain state says block `n - RetentionPeriod` stored transactions, but no
`check_proof` extrinsic was in the block. So ask whether a proof can be built from this database
at all:

```bash
tx-index-tool proof <db> --current --retention-period 100800 --live
```

It resolves the target the way the node does, then proves it:

```
authoring #1990075, retention 100800 → proving #1889275, randomness = hash(#1990074)

Storage proof for block #1889275
  total chunks in block:    2
  tx content hash:          0x3f7ee984…9b38
  local verification:       OK
```

| What you see | What it means |
| --- | --- |
| `local verification: OK` | a proof *can* be built here — the fault is elsewhere: wrong target, or the failing node has a different database |
| `no indexed body to prove at block #N` (exit 3) | the client emits **no** proof for that height. If on-chain state says that block stored transactions, that mismatch is what trips the assertion |
| `col11 missing value for hash 0x…` | the data is gone while a block still references it. If the entry was being auto-renewed, start with the pruning-cadence scenario below; otherwise see the refcount and corruption scenarios |
| `local verification: FAILED` (exit 2) | the value is present but does not match what the proof attests to |
| `chain agreement: MISMATCH` (exit 2) | with `--expect-root`: the bytes on disk are not the ones the chain committed to — see below |

Get the retention period from the chain rather than guessing it — the tool reads raw columns and
cannot execute the runtime API that holds it:

```bash
curl -sH 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"state_call","params":["TransactionStorageApi_retention_period","0x"]}' \
  http://127.0.0.1:9944   # SCALE u32, little-endian: 0x40890100 = 100800
```

Then check whether the target block has an indexed body at all. A node that warp-synced or was
restored from a snapshot has no historical indexed bodies and never emits a proof for them.

```bash
tx-index-tool block <db> 1889275 --live
```

Run it **on the node that is failing**. A full node holding the data says nothing about the
collator that has to author.

### Checking the proof against the chain, not just against itself

By default the proof and the root it is checked against are both derived from the same bytes on
disk, so the check is only internal consistency: a value that has since been replaced still
verifies against its own recomputed root. `--expect-root` closes that gap by comparing the
recomputed root with the one the chain committed to when the data was stored.

```bash
tx-index-tool proof <db> 1889275 --expect-root 0x9b3a…c41f --live
```

```
  tx chunk root:            0x9b3a…c41f
  local verification:       OK
  expected chunk root:      0x9b3a…c41f
  chain agreement:          OK — the stored bytes are what the chain committed to
```

A `MISMATCH` means the bytes on disk are not the bytes the entry was stored with — the proof is
internally fine but the runtime would reject it. That is exit 2, same as a failed local
verification.

The expected root is `TransactionInfo::chunk_root` from on-chain state, held in
`TransactionStorage::Transactions(block_number)` — a `Vec<TransactionInfo>` for the block that
stored (or last renewed) the data. Each element starts with `chunk_root` (32 bytes) followed by
`content_hash` (32 bytes), so the root is the 32 bytes immediately preceding the content hash
`proof` reports as `tx content hash`. Read it at a block hash the entry was still alive at:

```bash
# key = twox128("TransactionStorage") ++ twox128("Transactions")
#       ++ blake2_128(block_le) ++ block_le
curl -sH 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"state_getStorage","params":["0x<key>","0x<at>"]}' \
  http://127.0.0.1:9944
```

`trace --rpc-url` reads the same value and prints the command with the root filled in, which
avoids building the key by hand.

## Renewed data vanished while it was still being auto-renewed

Symptom: `Missing indexed transaction 0x…` for an entry the chain has been renewing all along.
`incident sdk-12106 drift` reports nothing here — every reference and release was symmetric.

Cause: `--blocks-pruning` at or below the renewal cadence. Renewals land `RetentionPeriod + 1`
blocks apart, so the node must retain block `S` until block `S+RP+1` is imported and has taken
its reference. Below that, the last reference is released before the renewal that would have
carried it forward, both col11 rows are deleted, and the renewal — which carries only the hash —
cannot restore them.

```bash
tx-index-tool trace <db> 0x<hash> --live --rpc-url wss://archive-node --probe-cadence
```

`DANGLING`, with a long run of `← reference released here` rows the chain still has entries for,
is the signature.

Required: `--blocks-pruning > RetentionPeriod + 1` on any node expected to author. `RP+1` is the
exact boundary and holds only while renewals stay on cadence; use margin or archive. Raising
`RetentionPeriod` on chain lengthens the cadence without changing the flag.

Fixing the flag stops further loss but restores nothing. Recover by `repair` per entry from a
node that still has the value, or by resyncing with the corrected setting — **full sync, not
warp or fast**, since only executing the original `store` block puts the payload on disk.

The hole opens for every auto-renewed entry at its own boundary, so check the scope before
repairing one at a time:

```bash
tx-index-tool diff <collator-db> <archive-db> --blocks
```

## Data is missing, or a hash will not resolve

Ask whether any stored value no longer hashes to the key it is filed under:

```bash
tx-index-tool list <db> --corrupted-only --live
```

```
Value entries:         112336  (32.81 GiB total)
Unexpected key rows:   0
Integrity:             112336 verified, 0 corrupted  (every value hashes to its key)
No entries matched.
```

Exit 0 means clean. For each failure you get the size, the refcount, what the bytes actually
hash to under all three algorithms, and **every block still referencing it**.
`Unexpected key rows` counts col11 rows that are neither a 32-byte value nor a `hash‖0x00`
counter; anything above zero is a key shape this tool does not understand.

This reads every value in the column, so it takes a while — about 48 s for 32.8 GiB, and that
time is disk, not hashing. Narrow it with `--block N`, `--hash H` or `--min-size N` when you can.

## One entry is behaving oddly and you need its whole history

`trace` rebuilds the reference ledger for a single content hash: every block that holds a
reference, what each contributes, and how the sum compares with the counter on disk.

```bash
tx-index-tool trace <db> 0xe60057c3…b24a --live
```

```
Scan duration:         2.141129794s  (132115 BODY_INDEX entries)
Value on disk:         32768 bytes, blake2b256(value) == key
Counter on disk:       155

  block         Δ    cum  body shape       authored               chain
  #34884       +1      1  Indexed          2026-08-13 17:58:30 UTC
  #35885       +1      2  Indexed          2026-08-13 19:38:42 UTC
  …
  #42892       +1      9  MultiRenew(×1)   2026-08-14 07:25:24 UTC
  …
  #189038      +1    155  Indexed          2026-08-24 12:39:36 UTC

Alive references:      155   (sum over 155 referring block(s))

Result: CONSISTENT — the counter holds 155, matching the ledger.
```

Only the traced hash is ever hashed, so this costs one `BODY_INDEX` pass — seconds, not the
minutes a full `list` takes.

col11 holds one counter per entry, not a history, so the counter's value at some past block is
not recoverable. The ledger is, and the verdict follows from it:

| Verdict | Meaning |
| --- | --- |
| `CONSISTENT` | counter equals the references alive blocks carry |
| `COUNTER SHORT` | references were lost — the first block to prune can take the counter to zero while others still reference the value (see polkadot-sdk#12106 below) |
| `COUNTER EXCESS` | releases were missed, so the value will never be reclaimed |
| `DANGLING` | alive blocks reference a hash with **no value stored**. A renewal cannot repair it: the extrinsic carries only the hash, and referencing a missing counter is a silent no-op. Any block whose proof targets one of those blocks cannot be authored |
| `ABSENT` | nothing stored and nothing referencing it (exit 3) |

### Cross-checking against the chain

The database only knows which references *survive* in it. Add `--rpc-url` and each block gets
the chain's account of itself, which is what separates a reference that was legitimately pruned
from one released early:

```bash
tx-index-tool trace <db> 0xe60057c3…b24a --live \
  --rpc-url wss://your-node.example --probe-cadence
```

```
Value on disk:         <absent>
Counter on disk:       <absent>
Chain:                 wss://…  head #190037, finalized #190037, RetentionPeriod 1000
Chain location:        Transactions(#189038) index 0   next proof due at #190038

  block         Δ    cum  body shape             authored               chain
  #34884        -      -  not in this database                          entry TransactionStorage.Stored   ← reference released here
  #35885        -      -  not in this database                          entry DataRenewal.DataRenewed   ← reference released here
  …
  #188037       -      -  not in this database                          entry DataRenewal.DataRenewed   ← reference released here
  #189038      +1      1  Indexed                2026-08-24 12:39:36 UTC entry DataRenewal.DataRenewed

Alive references:      1   (sum over 1 referring block(s))

Result: DANGLING — 1 alive block(s) reference this hash and no value is stored.

The chain recorded a reference at 154 block(s) this database no longer holds:
  #34884, #35885, #36886, … (+124 more)
  Normal if those blocks were pruned. If any is inside the pruning window, its reference was
  released early.
```

`--probe-cadence` walks the renewal cadence (`RetentionPeriod + 1`) outwards from the blocks
already known, which is how renewals this database no longer references get found;
`--chain-max-blocks` bounds the round-trips.

Two properties of the chain column:

- Auto-renewals fire in `on_initialize` and have no extrinsic, so events are what gets reported:
  `TransactionStorage.Stored`, `DataRenewal.DataRenewed`, `DataRenewal.RenewalFailed`.
- `Transactions(N)` is read at block `N`'s own state. A node with pruned state reports `state
  pruned`, not `no entry` — silence there is not evidence of absence.

When the chain supplies a `chunk_root`, the trace prints a ready-made `proof --expect-root`
command for it.

## Refcounts are wrong (polkadot-sdk#12106)

Before that fix, kvdb collapsed N same-key refcount operations in one transaction into a single
±1, so a counter reads "one per referencing block" where it should read "one per reference".
Left alone, the first block to prune decrements by its whole occurrence count, takes the counter
to zero, and the value is deleted while the remaining blocks still reference it.

```bash
tx-index-tool incident sdk-12106 drift <db> --live     # analyse
tx-index-tool incident sdk-12106 drift <db> --apply    # backfill, node stopped
tx-index-tool incident sdk-12106 drift <db> --live     # confirm clean (exit 0)
```

The analysis separates harmless cases (sole referrer, so over-release saturates) from at-risk
ones (more than one referring block), and shows what each counter should hold:

```
Top 10 on-disk-drifted counters (current → correct):
  0x72a52a91…92e8  10 → 4500  (+4490)
```

`10 → 4500` across 10 referencing blocks is the collapse signature: one per block where it should
be one per reference. Only the counter row is written; stored values are never touched, and
`--apply` is refused with `--live`.

This is a kvdb-only fault — ParityDB reference-counts col11 natively, so the collapse cannot
happen there.

## Values whose bytes were cut in the wrong place (polkadot-bulletin-chain#574)

That PR appended a `(MultiSigner, MultiSignature, u64)` tuple such that the boundary between
`BODY_INDEX.header` and the col11 value landed 106 bytes off (107/108 for Ecdsa). The original
bytes are all still on disk, just cut in the wrong place, so they can be recovered with no
external copy:

```bash
tx-index-tool incident bulletin-574 realign <db> --live             # dry run, every bad entry
tx-index-tool incident bulletin-574 realign <db> --hash 0x… --live  # one entry
tx-index-tool incident bulletin-574 realign <db> --apply            # write, node stopped
```

It reconstructs `header ++ col11_value` and searches for the split whose data side hashes to the
slot key: the known 106/107/108 sizes first, then generic chop-from-end, start-shift and
length-preserving window shifts within `--max-shift` (default 200). The report gives start shift,
end chop, matching algorithm and corrected size per entry, and groups recoveries by pattern so a
uniform bug collapses to one line.

**Run `verify` before repairing.** A body is reassembled as exactly `header ++ col11`
(sc-client-db `body_uncached`), while the authored extrinsic was `header ++ data ++
trailing-fields`. For any call with fields after its data — the pre-#574 `HopPromotion::promote`
shape — integrity and executability are mutually exclusive:

| col11 holds | hash | body reassembles to the authored bytes |
| --- | --- | --- |
| the aligned data | ✅ | ❌ block permanently unexecutable |
| the trailing window | ❌ | ✅ still replays |

`realign --apply` buys the first row and destroys the second, *and* discards the trailing fields,
which exist only in the value it overwrites. There is no pair of values that satisfies both, so
writing both rows does not help. `verify` tells the two states apart, and identifies databases a
previous repair already left unexecutable — those show as `col11-only repair (hash ok, NOT
executable)` and are invisible to `list --corrupted-only`, since their values hash correctly.

Single renewals are stored as `Indexed { hash, header: <the whole extrinsic> }`, so their header
is complete and their value belongs to an earlier block. `verify` recognises and skips them.

## Restoring a value you have a good copy of

```bash
tx-index-tool repair <db> 0x<hash> good-bytes.bin --live    # plan
tx-index-tool repair <db> 0x<hash> good-bytes.bin --apply   # write, node stopped
```

The write is refused unless `algo(new_data) == hash`, and the counter row is never touched.
`--algo` defaults to `blake2b256`; if it is wrong the plan reports what the on-disk value hashes
to under all three algorithms and which, if any, reproduces the key.

On a `DANGLING` entry the counter row is absent as well. Writing the value is enough to restore
authoring, since the proof path reads only the value, but with no counter the entry is never
reclaimed by pruning. Verify the write against the chain rather than against itself:

```bash
tx-index-tool proof <db> <block> --expect-root 0x<chunk_root> --live
```

`trace --rpc-url` prints that command with the root filled in.

## What is in here, and how much of it?

```bash
tx-index-tool list <db> --limit 0 --preview 0 --no-blocks --live  # everything, fastest form
tx-index-tool list <db> --sort size --desc --limit 20 --live      # the largest payloads
tx-index-tool list <db> --min-size 1000000 --live                 # only multi-MB entries
```

The header gives entry count, total bytes and the column-wide integrity result. `--no-blocks`
skips the `BODY_INDEX` pass, dropping the created/last-seen columns and roughly halving the work.

## The life of one entry

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

When it was stored, when it was last renewed (a second `last seen` line appears once something
renews it), how many blocks reference it, its refcount, and enough bytes to recognise the
payload — `eroots` above is a CAR header, and `01 70` a dag-pb CID prefix. The integrity line
also tells you which hashing scheme the CID used: `blake2b256` for plain `store`,
`sha2_256`/`keccak256` via `store_with_cid_config`.

A `--hash` or `--block` filter is resolved by point lookup, skipping the column walk entirely:
about 9 s instead of 48 s on a 32.8 GiB column, and nearly all of that is opening the secondary.

## What did block N do?

```bash
tx-index-tool block <db> 1889275 --live         # what the body declared
tx-index-tool list  <db> --block 1889275 --live # the state of the data it points at
```

Different questions, and neither subsumes the other:

- **`block N`** is block-centric: the extrinsic mix (`1 Indexed, 0 MultiRenew, 2 Full`) and the
  per-hash *body shape* — `Indexed`, `3×Indexed`, `MultiRenew(×4)`, `2×Indexed + MultiRenew(×3)`.
  That multiplicity is the refcount-bug signal. It also prints the block hash even when there is
  no indexed body, which is how you fetch a parent hash for `proof --random`.
- **`list --block N`** is entry-centric: integrity, algorithm, refcount, first and last
  referencing block with times, and a preview — chain-wide facts the block cannot tell you.

## Retention, expiry and leaks

```bash
tx-index-tool list <db> --from-block 1880000 --to-block 1890000 --live  # stored in a window
tx-index-tool list <db> --orphans-only --live                           # nothing references these
```

`--from-block`/`--to-block` bound where an entry was *stored* (its first referencing block), so
they need the `BODY_INDEX` pass and are rejected alongside `--no-blocks`. Orphans — entries no
alive block references any more — never fall inside a range, and are either pruning candidates or
entries that outlived their references.

## Comparing two nodes

When one node can author and the other cannot, or one serves data the other doesn't, the delta
the delta between their databases is what to look at:

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

Three comparisons, each catching a different failure:

- **Entry sets** — what one node holds and the other doesn't. Answers "did the collator lose the
  data".
- **Refcounts per shared entry** — spots a database where the `sdk-12106` backfill hasn't been
  applied, or where pruning diverged (`refcount     A 4500   B 10`).
- **`--blocks`** — which blocks have an indexed body on one node but not the other. Reach for
  this first on a proof stall: a collator missing the body index for the proof target emits no
  proof, and this names those blocks directly.

Values are compared through their keys, not their bytes: col11 is content-addressed, so two
entries that verify under the same key are identical, and a differing size or a one-sided
integrity failure is the signal. Both sides are opened read-only, so `--live` works against a
running node — including comparing a live collator with a stopped one.

Exit 0 means identical, 2 means they differ. `--limit N` caps the per-entry lines (0 prints all);
the summary counts always describe the whole comparison, not just the printed rows.

# Command reference

| Command | What it does |
| --- | --- |
| `list <db>` | Stored data: size, hash algorithm, refcount, creating block and time, a `hexdump -C` preview, and an integrity check per entry |
| `block <db> <n>` | Which stored transactions one block's body references, with their on-disk state |
| `proof <db> <n> [--random <hex>] [--expect-root <hex>]` | Recomputes the storage proof the inherent provider emits, and verifies it; `--expect-root` also compares against the root the chain recorded |
| `proof <db> --current`&nbsp;/&nbsp;`--authoring N` `--retention-period R` | Same, resolving target and randomness the way the node does |
| `trace <db> <hash> [--rpc-url URL] [--probe-cadence]` | Rebuilds one entry's reference ledger — every referring block, its contribution, and the verdict on the counter; with a URL, cross-checks each block against chain state and events |
| `diff <db> <other> [--blocks] [--limit N]` | Compares two databases: entries only one has, refcount, size and integrity disagreements, and optionally which blocks have an indexed body |
| `repair <db> <hash> <file> [--algo N] [--apply]` | Overwrites one corrupted value with known-good bytes |
| `incident sdk-12106 drift <db> [--apply]` | Counters the kvdb refcount collapse left short; `--apply` sets each to its true reference count |
| `incident bulletin-574 verify <db>` | Classifies every indexed entry's `BODY_INDEX.header` ↔ col11 seam: healthy, single renewal, original mis-split, or a col11-only repair that left the block unexecutable |
| `incident bulletin-574 realign <db> [--hash H] [--max-shift N] [--apply]` | Recovers values whose `BODY_INDEX.header ++ col11` split moved |

Faults tied to one past incident live under `incident`, named for the pull request behind them,
because they diagnose a particular bug rather than answer a general question about the database.

`list` filters: `--limit N` (0 = all), `--preview N` bytes, `--sort block|size|refcount|hash`,
`--desc`, `--corrupted-only`, `--orphans-only`, `--min-size N`, `--hash <hex>`, `--block N`,
`--from-block N`, `--to-block N`, `--no-blocks`.

Every command accepts `--live` or `--secondary <dir>`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Clean, or the query found what it was looking for |
| 2 | Findings: drifted counters, a corrupted entry, a failed proof, an unrecoverable entry |
| 3 | A specific block or hash was asked about and is not there |
| 64 | Usage error |
| 74 | I/O failure, including a database that cannot be opened |

Exit 3 is only for *targeted* misses. A predicate that matches nothing — `--corrupted-only` on a
healthy database — is success, so that command is safe to use as a monitoring check.

## Limits and gotchas

- **Writes need the node stopped.** `--apply` is refused with `--live`/`--secondary`. Rehearse a
  repair on a copy first; the matching read-only command afterwards is the check.
- **The retention period must be supplied.** The tool reads raw columns and cannot call the
  runtime API, so `proof --current` needs `--retention-period` (see the RPC above).
- **Proof verification is local unless you say otherwise.** On its own it answers "would a proof
  built from this database be internally consistent", not "does this match what the chain
  accepted" — pass `--expect-root` for the second question.
- **`blocks_pruning > RetentionPeriod + 1` is required and unenforced.** Neither node nor runtime
  validates it; the violation stays invisible for `RetentionPeriod` blocks and then surfaces as
  an unauthorable block.
- **A renewal against data the node lacks succeeds on chain and writes nothing to col11.**
  `store_or_reference` falls to `tx.reference(...)`, which is a no-op when the counter is absent,
  while `BODY_INDEX` records the reference anyway. `sp_io::transaction_index::renew` returns
  `()`, so nothing reports it. `trace` shows the result as `DANGLING`.
- **`DANGLING` does not heal.** col11 is written only by a `store` extrinsic carrying the payload
  inline, or by `renew_payloads` supplied through `BlockImportParams` at import. A renew
  extrinsic carries only the hash.
- **RocksDB only** for now — see below.
- **File descriptors:** RocksDB wants thousands, unlimited in `--live` mode. The tool raises its
  own soft limit at startup; if that fails it says so, and `ulimit -n 65536` is the fallback.
- **Schema version:** the column indices are an on-disk contract, so the tool compares the
  `db_version` file beside the data against the version it was written for (4) and warns on a
  mismatch.
- **Timestamps are heuristic.** They come from each block's `Timestamp::set` inherent, accepted
  as a bare extrinsic whose call is one range-checked `Compact<u64>` and nothing else. No call
  indices are hardcoded, but the reports label it as a heuristic.

## ParityDB

Not supported yet. When it is, the shape will be:

- **Works:** `block`, `proof`, `list --block/--hash` — all point lookups. A full `list` is also
  possible: `iter_column_while` yields each value with its native refcount, and because col11 is
  content-addressed the hash is recomputable by hashing the value, which is the integrity check
  anyway.
- **Degraded:** a *corrupted* entry's key cannot be named with certainty — hashing its value
  yields nothing, and the stored index key is siphashed above byte 16 in the current format.
  `BODY_INDEX` keys are one-way hashed too, so created/last-seen and orphan detection would need
  a walk over block *numbers* through `KEY_LOOKUP` rather than over database keys.
- **Refused:** everything that writes col11, and both `incident` commands. On a `ref_counted`
  column a `Set` on an existing key only increments the refcount and *discards the supplied
  value* (`// Replace is not supported`), so a repair would silently change nothing while bumping
  the count. `sdk-12106` cannot apply there in any case, since ParityDB counts references itself
  and there is no counter row to collapse.

ParityDB never verifies that a value hashes to its key —
`preimage: true` is a declaration, not a check — so this class of corruption is equally possible
there, and re-storing the correct bytes does *not* heal it.
