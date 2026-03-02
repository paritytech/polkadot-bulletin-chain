/**
 * Substrate JSON-RPC mock helpers for E2E tests.
 *
 * Uses Playwright's `page.routeWebSocket()` to intercept WebSocket connections
 * from polkadot-api and provide deterministic responses.
 */
import type { Page } from "@playwright/test";

/**
 * Block all WebSocket connections. The connection appears open to the client
 * but no data flows. The app stays in "connecting" state.
 *
 * Use this for tests that verify UI structure, navigation, and interactions
 * that don't require an active chain connection.
 */
export async function blockWebSockets(page: Page): Promise<void> {
  await page.routeWebSocket(/.*/, () => {
    // Connection is intercepted and appears open, but no messages are handled.
    // polkadot-api sends chainHead_v1_follow but never gets a response.
  });
}

export interface MockChainConfig {
  chainName?: string;
  specVersion?: number;
  tokenSymbol?: string;
  tokenDecimals?: number;
  blockNumber?: number;
}

const DEFAULT_CONFIG: Required<MockChainConfig> = {
  chainName: "bulletin-westend",
  specVersion: 1,
  tokenSymbol: "BUL",
  tokenDecimals: 12,
  blockNumber: 42,
};

/**
 * Mock Substrate JSON-RPC over WebSocket.
 *
 * Provides minimal responses for the chainHead v1 protocol so polkadot-api
 * considers the connection alive. Note: this mock does NOT provide real
 * SCALE-encoded metadata, so `api.constants.*` calls will fail. The app's
 * catch blocks handle this gracefully — status reaches "connected" but
 * chain info fields (chainName, specVersion, etc.) remain empty.
 *
 * Use `blockWebSockets()` for simpler tests that only need UI structure.
 * Use this for tests that need the app to attempt a connection.
 */
export async function mockSubstrateRpc(
  page: Page,
  config: MockChainConfig = {},
): Promise<void> {
  const cfg = { ...DEFAULT_CONFIG, ...config };

  await page.routeWebSocket(/.*/, (ws) => {
    const subscriptionId = "mock-follow-sub";
    let operationCounter = 0;
    const blockHash = "0x" + "ab".repeat(32);

    ws.onMessage((raw) => {
      let data: { id: number; method: string; params?: unknown[] };
      try {
        data = JSON.parse(raw.toString());
      } catch {
        return;
      }

      const { id, method } = data;

      const respond = (result: unknown) => {
        ws.send(JSON.stringify({ jsonrpc: "2.0", id, result }));
      };

      const respondError = (code: number, message: string) => {
        ws.send(
          JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } }),
        );
      };

      const sendFollowEvent = (event: unknown) => {
        ws.send(
          JSON.stringify({
            jsonrpc: "2.0",
            method: "chainHead_v1_followEvent",
            params: { subscription: subscriptionId, result: event },
          }),
        );
      };

      switch (method) {
        case "rpc_methods":
          respond({
            methods: [
              "chainHead_v1_follow",
              "chainHead_v1_unfollow",
              "chainHead_v1_call",
              "chainHead_v1_storage",
              "chainHead_v1_unpin",
              "chainHead_v1_body",
              "chainHead_v1_header",
              "system_properties",
            ],
          });
          break;

        case "chainHead_v1_follow":
          respond(subscriptionId);

          // Send initialized event
          setTimeout(() => {
            sendFollowEvent({
              event: "initialized",
              finalizedBlockHashes: [blockHash],
              finalizedBlockRuntime: {
                type: "valid",
                spec: {
                  specName: cfg.chainName,
                  implName: cfg.chainName,
                  specVersion: cfg.specVersion,
                  implVersion: 0,
                  transactionVersion: 1,
                  apis: {},
                },
              },
            });
          }, 10);

          // Send new block + best block events
          setTimeout(() => {
            const newBlockHash = "0x" + "cd".repeat(32);
            sendFollowEvent({
              event: "newBlock",
              blockHash: newBlockHash,
              parentBlockHash: blockHash,
              newRuntime: null,
            });
            sendFollowEvent({
              event: "bestBlockChanged",
              bestBlockHash: newBlockHash,
            });
          }, 50);
          break;

        case "chainHead_v1_unfollow":
          respond(null);
          break;

        case "chainHead_v1_call": {
          const opId = `call-${++operationCounter}`;
          respond({ result: "started", operationId: opId });

          // Return operationError for all calls — we can't provide real
          // SCALE-encoded metadata. The app catches these errors gracefully.
          setTimeout(() => {
            sendFollowEvent({
              event: "operationError",
              operationId: opId,
              error: "Mock: runtime calls not available",
            });
          }, 10);
          break;
        }

        case "chainHead_v1_storage": {
          const opId = `storage-${++operationCounter}`;
          respond({ result: "started", operationId: opId });

          setTimeout(() => {
            sendFollowEvent({
              event: "operationStorageDone",
              operationId: opId,
            });
          }, 10);
          break;
        }

        case "chainHead_v1_unpin":
          respond(null);
          break;

        case "chainHead_v1_body": {
          const opId = `body-${++operationCounter}`;
          respond({ result: "started", operationId: opId });

          setTimeout(() => {
            sendFollowEvent({
              event: "operationBodyDone",
              operationId: opId,
              value: [],
            });
          }, 10);
          break;
        }

        case "chainHead_v1_header":
          respondError(-32603, "Mock: header not available");
          break;

        case "system_properties":
          respond({
            tokenSymbol: cfg.tokenSymbol,
            tokenDecimals: cfg.tokenDecimals,
          });
          break;

        default:
          respondError(-32601, `Unsupported method: ${method}`);
          break;
      }
    });
  });
}
