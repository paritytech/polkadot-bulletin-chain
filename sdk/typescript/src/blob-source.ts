// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Re-openable byte source for streamed uploads.
 *
 * The contract that matters: `open()` must be callable MORE THAN ONCE and
 * yield the SAME bytes each time. That re-readability is what lets the
 * hash/estimate/dedup pass and the submission pass share one source without
 * buffering the whole file — see `estimateUpload(source)` / `submit(estimate, source)`.
 *
 * A one-shot `ReadableStream` does NOT satisfy this; wrap a re-opener instead
 * (e.g. `blobFromFactory(() => fs.createReadStream(path))`). Node's `fs` is not
 * imported here so the browser bundle stays clean — callers supply the opener.
 */
export interface BlobSource {
  /** Total byte length if known up front. Lets `estimateUpload` size
   *  authorization without a full pass; omit only for unknown-length streams. */
  readonly size?: number
  /** Open a fresh forward read from the start. Re-callable. */
  open(): AsyncIterable<Uint8Array>
}

/**
 * A {@link BlobSource} that also supports random-access byte-range reads. Lazy
 * submission needs this: it fetches chunk `i` via `read(plan.offsets[i],
 * plan.chunkSizes[i])` instead of holding the whole source in memory. A file
 * satisfies both halves (stream for the estimate pass, range-read for submit);
 * a forward-only stream satisfies only `BlobSource` and must be buffered.
 */
export interface SeekableSource extends BlobSource {
  readonly size: number
  /** Read exactly `length` bytes starting at `offset`. */
  read(offset: number, length: number): Promise<Uint8Array>
}

/** In-memory bytes as a {@link SeekableSource} — `read` is a zero-copy
 *  subarray; also streamable for the estimate pass. */
export function blobFromBytes(data: Uint8Array): SeekableSource {
  return {
    size: data.length,
    async *open() {
      yield data
    },
    async read(offset: number, length: number) {
      return data.subarray(offset, offset + length)
    },
  }
}

/** A re-openable stream factory (Node `Readable` or Web `ReadableStream` are
 *  both `AsyncIterable<Uint8Array>`). `size` is optional but recommended. */
export function blobFromFactory(
  open: () => AsyncIterable<Uint8Array>,
  size?: number,
): BlobSource {
  return { size, open }
}

/**
 * A {@link SeekableSource} over the concatenation of `items` — the source for
 * the items-as-is submission path. Reads are item-aligned (the plan's offsets
 * are item boundaries), so `read` returns the matching item's bytes without
 * copying; it also handles spanning reads for safety. The items stay resident
 * (a batch is in memory anyway), so this is a zero-copy view, not a buffer.
 */
export function blobFromItems(
  items: ReadonlyArray<{ data: Uint8Array }>,
): SeekableSource {
  const offsets: number[] = []
  let total = 0
  for (const it of items) {
    offsets.push(total)
    total += it.data.length
  }
  return {
    size: total,
    async *open() {
      for (const it of items) yield it.data
    },
    async read(offset: number, length: number) {
      const exact = offsets.indexOf(offset)
      if (exact !== -1 && items[exact]!.data.length === length) {
        return items[exact]!.data
      }
      const out = new Uint8Array(length)
      let written = 0
      let pos = offset
      while (written < length) {
        let idx = 0
        while (idx + 1 < offsets.length && offsets[idx + 1]! <= pos) idx++
        const itemData = items[idx]!.data
        const within = pos - offsets[idx]!
        const take = Math.min(length - written, itemData.length - within)
        out.set(itemData.subarray(within, within + take), written)
        written += take
        pos += take
      }
      return out
    },
  }
}

/** Read a BlobSource fully into one Uint8Array. O(size) memory — for callers
 *  that need the whole buffer at once; the submission path reads lazily via
 *  `SeekableSource.read` instead. */
export async function collectBlob(source: BlobSource): Promise<Uint8Array> {
  const parts: Uint8Array[] = []
  let total = 0
  for await (const part of source.open()) {
    parts.push(part)
    total += part.length
  }
  const out = new Uint8Array(total)
  let offset = 0
  for (const part of parts) {
    out.set(part, offset)
    offset += part.length
  }
  return out
}
