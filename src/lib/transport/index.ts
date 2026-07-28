/**
 * Transport abstraction layer.
 *
 * The server edition always uses WebSocket JSON-RPC.
 */
import { dbg } from "$lib/utils/debug";
import { WsTransport } from "./websocket";

export interface Transport {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(event: string, handler: (payload: T) => void): Promise<() => void>;
  isDesktop(): boolean;
  /** Subscribe to a run's real-time events (WS only, no-op on desktop) */
  subscribeRun(runId: string, lastSeq?: number): void;
  /** Unsubscribe from a run's events (WS only, no-op on desktop) */
  unsubscribeRun(runId: string): void;
}

let _instance: Transport | null = null;

export function getTransport(): Transport {
  if (!_instance) {
    _instance = new WsTransport();
    dbg("transport", "initialized", { type: "websocket" });
  }
  return _instance;
}
