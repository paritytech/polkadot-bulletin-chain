import { useCallback } from "react";
import type { UploadEvent } from "@parity/bulletin-sdk";
import { UploadStatus } from "@parity/bulletin-sdk";

/**
 * Progress handler for `client.upload()` / `client.uploadFile()` callbacks.
 * Maps per-item UploadEvent into a human-readable status string. For a
 * single-item upload the `(N/M)` prefix is dropped to keep the line short.
 */
export function useUploadProgressHandler(
  setTxStatus: (status: string) => void,
): (event: UploadEvent) => void {
  return useCallback(
    (event: UploadEvent) => {
      const prefix = event.total > 1 ? `(${event.index + 1}/${event.total}) ` : "";
      switch (event.type) {
        case UploadStatus.ItemStarted:
          setTxStatus(`${prefix}Broadcasting...`);
          break;
        case UploadStatus.ItemInBlock:
          setTxStatus(`${prefix}In block #${event.blockNumber}...`);
          break;
        case UploadStatus.ItemFinalized:
          setTxStatus(
            event.total > 1 && event.index + 1 < event.total
              ? `${prefix}Finalized @ #${event.blockNumber}`
              : "Finalized!",
          );
          break;
        case UploadStatus.ItemFailed:
          setTxStatus(`${prefix}Failed: ${event.error.message}`);
          break;
      }
    },
    [setTxStatus],
  );
}
