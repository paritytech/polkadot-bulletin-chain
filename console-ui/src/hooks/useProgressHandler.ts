import { useCallback } from "react";
import type { ProgressEvent } from "@parity/bulletin-sdk";
import { TxStatus } from "@parity/bulletin-sdk";

/**
 * Shared progress callback handler for SDK transaction status updates.
 * Maps TxStatus events to user-friendly status strings.
 */
export function useProgressHandler(
  setTxStatus: (status: string) => void,
): (event: ProgressEvent) => void {
  return useCallback(
    (event: ProgressEvent) => {
      console.log("SDK progress:", event);
      switch (event.type) {
        case TxStatus.Signed:
          setTxStatus("Transaction signed...");
          break;
        case TxStatus.Broadcasted:
          setTxStatus("Broadcasting to network...");
          break;
        case TxStatus.InBlock:
          setTxStatus(`Included in block #${event.blockNumber}...`);
          break;
        case TxStatus.Finalized:
          setTxStatus("Finalized!");
          break;
        default:
          console.log("Unhandled progress event:", event.type);
          break;
      }
    },
    [setTxStatus],
  );
}
