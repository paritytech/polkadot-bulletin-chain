import { useCallback } from "react";
import type { ProgressEvent } from "@bulletin/sdk";
import { TxStatus } from "@bulletin/sdk";

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
      if (event.type === TxStatus.Signed) {
        setTxStatus("Transaction signed...");
      } else if (event.type === TxStatus.Broadcasted) {
        setTxStatus("Broadcasting to network...");
      } else if (event.type === TxStatus.InBlock) {
        setTxStatus(`Included in block #${event.blockNumber}...`);
      } else if (event.type === TxStatus.Finalized) {
        setTxStatus("Finalized!");
      }
    },
    [setTxStatus],
  );
}
