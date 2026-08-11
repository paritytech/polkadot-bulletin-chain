// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

/**
 * Submission strategies for pipelineStore.
 *
 * The interface (`SubmissionStrategy`) abstracts how signed extrinsics
 * reach the chain. Today only `nonce-tracking` is implemented:
 * `author_submitExtrinsic` fan-out across every configured submit client,
 * with the chainHead reconciler responsible for inclusion / hijack
 * detection via per-item nonce + TBCH lookup. The abstraction is kept so
 * alternative strategies (e.g. `transactionWatch_v1`-based) can be
 * plugged in without touching the pipeline.
 */

import type { SubstrateClient } from "@polkadot-api/substrate-client"

// ---------------------------------------------------------------------------
// Substrate author RPC error codes (mirror of the AuthorRpcError used by
// pipeline.ts). Duplicated here so this file stands alone — the strategy
// can be tested without pulling pipeline.ts in.
// ---------------------------------------------------------------------------

enum AuthorRpcError {
  InvalidTransaction = 1010,
  UnknownValidity = 1011,
  TemporarilyBanned = 1012,
  AlreadyImported = 1013,
  TooLowPriority = 1014,
  CycleDetected = 1015,
  ImmediatelyDropped = 1016,
  InvalidTransactionV2 = 1017,
  UnauthorizedTransaction = 1018,
  UnknownCustomValidity = 1019,
  UnknownBuiltinValidity = 1020,
  FutureTransaction = 1021,
}

type RpcErrorClass = "terminal" | "retryable" | "already_imported" | "unknown"

function classifyAuthorRpcError(code: number | undefined): RpcErrorClass {
  switch (code) {
    case AuthorRpcError.InvalidTransaction:
    case AuthorRpcError.UnknownValidity:
    case AuthorRpcError.CycleDetected:
    case AuthorRpcError.InvalidTransactionV2:
    case AuthorRpcError.UnauthorizedTransaction:
    case AuthorRpcError.UnknownCustomValidity:
    case AuthorRpcError.UnknownBuiltinValidity:
      return "terminal"
    case AuthorRpcError.TemporarilyBanned:
    case AuthorRpcError.TooLowPriority:
    case AuthorRpcError.ImmediatelyDropped:
    case AuthorRpcError.FutureTransaction:
      return "retryable"
    case AuthorRpcError.AlreadyImported:
      return "already_imported"
    default:
      return "unknown"
  }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Selectable strategy on `ClientConfig`. Only one strategy is implemented
 * today; the union type leaves room to add others without changing the
 * `ClientConfig` shape. */
export type SubmissionStrategyKind = "nonce-tracking"

/** Per-item result returned by `broadcastWave`. */
export interface ItemBroadcastResult {
  /** True if at least one submission was accepted by the pool. */
  accepted: boolean
  /** When `accepted=false`, the RPC error code from the first retryable
   * rejection (1014 / 1016 / 1012 / 1021). Undefined means every
   * submission terminal-erred — caller should refresh the nonce. */
  retryableCode?: number
}

/** Wave-level summary returned by `broadcastWave`. */
export interface WaveResult {
  terminalCode?: number
  terminalMsg?: string
  retryableCount: number
  retryableLastCode?: number
  itemResults: ItemBroadcastResult[]
}

export interface BroadcastArgs {
  /** Hex-encoded signed extrinsics in wave order. */
  signed: string[]
  /** Item index for each entry in `signed`. */
  waveIndexes: number[]
  /** Wave-local counters; `txsBroadcast` / `broadcastErrors` are mutated. */
  counters: { txsBroadcast: number; broadcastErrors: number }
}

export interface SubmissionStrategy {
  broadcastWave(args: BroadcastArgs): Promise<WaveResult>
  /** Stop tracking an item — release any per-item resources. */
  onItemSettled(index: number): void
  /** Release every per-item resource. */
  teardown(): void
}

// ---------------------------------------------------------------------------
// Nonce-tracking: `author_submitExtrinsic` fan-out across submit clients.
// Each item is assigned a nonce by the pipeline and tracked via TBCH at
// every best-block reconcile; the strategy itself doesn't subscribe to
// per-tx events — it just submits and trusts the chainHead reconciler.
// ---------------------------------------------------------------------------

export interface NonceTrackingStrategyArgs {
  submitClients: SubstrateClient[]
}

export function createNonceTrackingStrategy(
  args: NonceTrackingStrategyArgs,
): SubmissionStrategy {
  const { submitClients } = args

  async function broadcastWave(b: BroadcastArgs): Promise<WaveResult> {
    const { signed, counters } = b
    let terminalCode: number | undefined
    let terminalMsg: string | undefined
    let retryableLastCode: number | undefined
    const accepted: boolean[] = new Array(signed.length).fill(false)
    const itemRetryableCode: Array<number | undefined> = new Array(
      signed.length,
    ).fill(undefined)

    const all: Promise<void>[] = []
    for (let k = 0; k < signed.length; k++) {
      const hex = signed[k] as string
      for (const client of submitClients) {
        all.push(
          client
            .request("author_submitExtrinsic", [hex])
            .then(() => {
              counters.txsBroadcast++
              accepted[k] = true
            })
            .catch((err: unknown) => {
              const e = err as { code?: number; message?: string }
              switch (classifyAuthorRpcError(e?.code)) {
                case "already_imported":
                  counters.txsBroadcast++
                  accepted[k] = true
                  return
                case "terminal":
                  counters.broadcastErrors++
                  if (terminalCode === undefined) {
                    terminalCode = e.code
                    terminalMsg = e.message
                  }
                  return
                default:
                  counters.broadcastErrors++
                  retryableLastCode = e?.code
                  if (itemRetryableCode[k] === undefined) {
                    itemRetryableCode[k] = e?.code
                  }
              }
            }),
        )
      }
    }
    await Promise.allSettled(all)

    const itemResults: ItemBroadcastResult[] = new Array(signed.length)
    let retryableCount = 0
    for (let k = 0; k < signed.length; k++) {
      if (accepted[k]) {
        itemResults[k] = { accepted: true }
      } else {
        itemResults[k] = {
          accepted: false,
          retryableCode: itemRetryableCode[k],
        }
        if (itemRetryableCode[k] !== undefined) retryableCount++
      }
    }
    return {
      terminalCode,
      terminalMsg,
      retryableCount,
      retryableLastCode,
      itemResults,
    }
  }

  return {
    broadcastWave,
    onItemSettled() {
      /* no-op — nonce-tracking keeps no per-item subscription state */
    },
    teardown() {
      /* no-op */
    },
  }
}
