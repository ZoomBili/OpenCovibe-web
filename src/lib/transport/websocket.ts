/**
 * WsTransport: WebSocket JSON-RPC transport for browser access.
 *
 * - Auto-reconnect with exponential backoff (1s -> 2s -> 4s -> ... -> 30s max)
 * - Close code 4401 -> stop reconnecting (auth failure)
 * - Request/response correlation via `id` field
 * - Server push events dispatched to registered handlers
 * - Cookie-based auth (no token in URL)
 * - Auto _subscribe/_unsubscribe for run-scoped events
 */
import { dbg, dbgWarn } from "$lib/utils/debug";
import type { Transport } from "./index";

interface PendingRequest {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
}

export class WsTransport implements Transport {
  private ws: WebSocket | null = null;
  private reqId = 0;
  private pending = new Map<string, PendingRequest>();
  private listeners = new Map<string, Set<(payload: unknown) => void>>();
  /** Per-run seq checkpoint for reconnect replay */
  private lastSeq = new Map<string, number>();
  /** Runs we've subscribed to on the server */
  private subscribedRuns = new Set<string>();
  private reconnectDelay = 1000;
  private shouldReconnect = true;
  private connectPromise: Promise<void> | null = null;

  private buildWsUrl(): string {
    const loc = window.location;
    const protocol = loc.protocol === "https:" ? "wss:" : "ws:";
    const url = `${protocol}//${loc.host}/ws`;
    dbg("transport", "ws.buildUrl", { url });
    return url;
  }

  private connect(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) return Promise.resolve();
    if (this.connectPromise) return this.connectPromise;

    // The promise must settle on EVERY terminal outcome (open, error, close,
    // timeout). Previously a refused connection only cleared `connectPromise`
    // in `onclose`, leaving every `await ensureConnected()` pending forever —
    // the whole UI stayed silently frozen even after the server came back.
    let resolveFn!: () => void;
    let rejectFn!: (error: Error) => void;
    const promise = new Promise<void>((resolve, reject) => {
      resolveFn = resolve;
      rejectFn = reject;
    });

    // Clear the shared slot once settled so the next caller can retry.
    const clearSlot = () => {
      if (this.connectPromise === promise) this.connectPromise = null;
    };
    promise.then(clearSlot, clearSlot);
    this.connectPromise = promise;

    let settled = false;
    const settle = (error?: Error) => {
      if (settled) return;
      settled = true;
      if (error) rejectFn(error);
      else resolveFn();
    };

    const url = this.buildWsUrl();
    dbg("transport", "ws.connecting", { url });

    const ws = new WebSocket(url);
    this.ws = ws;

    ws.onopen = () => {
      dbg("transport", "ws.connected");
      this.reconnectDelay = 1000;
      // Re-subscribe to all previously subscribed runs
      this.resubscribeAll();
      settle();
    };

    ws.onmessage = (ev) => {
      this.handleMessage(ev.data);
    };

    ws.onerror = (ev) => {
      dbgWarn("transport", "ws.error", ev);
      // `onclose` follows, but settle now so awaiting callers fail fast.
      settle(new Error("WebSocket connection failed"));
    };

    ws.onclose = (ev) => {
      dbg("transport", "ws.closed", { code: ev.code, reason: ev.reason });
      // Only mutate shared connection state if this is still the active socket.
      // A stale socket's close must not reject requests sent on its successor.
      const isCurrent = this.ws === ws;
      if (isCurrent) this.ws = null;

      // Reject the in-flight connect promise if the socket closed pre-open.
      // No-op after a successful `onopen` (already settled).
      settle(
        new Error(
          ev.code === 4401
            ? "WebSocket unauthorized (session expired)"
            : `WebSocket closed (code ${ev.code})`,
        ),
      );
      if (!isCurrent) return;

      // Reject all pending requests
      for (const [id, req] of this.pending) {
        req.reject(new Error(`WebSocket closed (code ${ev.code})`));
        this.pending.delete(id);
      }

      if (ev.code === 4401) {
        // Auth failure — stop reconnecting, redirect to login
        dbgWarn("transport", "ws.authFailure, redirecting to /login");
        this.shouldReconnect = false;
        window.location.href = "/login";
        return;
      }

      if (this.shouldReconnect) {
        const delay = Math.min(this.reconnectDelay, 30000);
        dbg("transport", "ws.reconnecting", { delay });
        setTimeout(() => {
          this.reconnectDelay = Math.min(this.reconnectDelay * 2, 30000);
          // Reconnect attempts are fire-and-forget; swallow the rejection
          // (the next invoke/listen will surface a real error if needed).
          this.ensureConnected().catch((e) => {
            dbgWarn("transport", "ws.reconnectFailed", e);
          });
        }, delay);
      }
    };

    // Timeout for initial connection. If the handshake is still pending after
    // 10s, close it and reject so callers don't hang.
    const timeoutId = setTimeout(() => {
      if (ws.readyState === WebSocket.CONNECTING) {
        settle(new Error("WebSocket connection timeout"));
        ws.close();
      }
    }, 10000);
    promise.then(
      () => clearTimeout(timeoutId),
      () => clearTimeout(timeoutId),
    );

    return promise;
  }

  private async ensureConnected(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) return;
    await this.connect();
  }

  /** Re-subscribe all tracked runs after reconnect */
  private resubscribeAll(): void {
    for (const runId of this.subscribedRuns) {
      const lastSeq = this.lastSeq.get(runId) ?? 0;
      dbg("transport", "ws.resubscribe", { runId, lastSeq });
      this.sendRaw({
        id: `req_${++this.reqId}`,
        method: "_subscribe",
        params: { run_id: runId, last_seq: lastSeq },
      });
    }
  }

  /** Subscribe to a run's real-time events on the server */
  subscribeRun(runId: string, lastSeq = 0): void {
    this.subscribedRuns.add(runId);
    // Monotonic: prevent checkpoint regression (e.g. accidental lastSeq=0 overwrites)
    const prev = this.lastSeq.get(runId) ?? 0;
    const effectiveSeq = Math.max(prev, lastSeq);
    this.lastSeq.set(runId, effectiveSeq);
    dbg("transport", "ws.subscribeRun", { runId, lastSeq, effectiveSeq });
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.sendRaw({
        id: `req_${++this.reqId}`,
        method: "_subscribe",
        params: { run_id: runId, last_seq: effectiveSeq },
      });
    }
  }

  /** Unsubscribe from a run's events */
  unsubscribeRun(runId: string): void {
    if (!this.subscribedRuns.has(runId)) return;
    this.subscribedRuns.delete(runId);
    this.lastSeq.delete(runId);
    dbg("transport", "ws.unsubscribeRun", { runId });
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.sendRaw({
        id: `req_${++this.reqId}`,
        method: "_unsubscribe",
        params: { run_id: runId },
      });
    }
  }

  private sendRaw(obj: Record<string, unknown>): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(obj));
    }
  }

  private handleMessage(raw: string): void {
    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse(raw);
    } catch {
      dbgWarn("transport", "ws.invalidJson", { raw: raw.slice(0, 200) });
      return;
    }

    // Response to a request (has `id` field)
    if (typeof msg.id === "string" && this.pending.has(msg.id)) {
      const req = this.pending.get(msg.id)!;
      this.pending.delete(msg.id);

      if (msg.error) {
        req.reject(new Error(String(msg.error)));
      } else {
        req.resolve(msg.result);
      }
      return;
    }

    // Server push event (has `event` field, no `id`)
    if (typeof msg.event === "string") {
      const event = msg.event as string;
      const payload = msg.payload;
      const seq = typeof msg.seq === "number" ? msg.seq : undefined;
      const runId = typeof msg.run_id === "string" ? (msg.run_id as string) : undefined;

      // Handle _full_reload (server signals client should reload a run)
      if (event === "_full_reload") {
        const reloadRunId = typeof msg.run_id === "string" ? msg.run_id : undefined;
        if (reloadRunId) {
          dbgWarn("transport", "ws._full_reload", { reloadRunId });
          this.lastSeq.delete(reloadRunId);
          this.subscribedRuns.delete(reloadRunId);
          const handlers = this.listeners.get("_full_reload");
          if (handlers) {
            for (const handler of handlers) handler({ run_id: reloadRunId });
          }
        }
        return;
      }

      // Track sequence checkpoint for reconnect replay
      if (seq !== undefined && runId) {
        const prev = this.lastSeq.get(runId) ?? 0;
        if (seq > prev) {
          this.lastSeq.set(runId, seq);
        }
      }

      // Inject _seq into bus-event payloads for session-store tracking
      if (event === "bus-event" && seq !== undefined && payload && typeof payload === "object") {
        (payload as Record<string, unknown>)._seq = seq;
      }

      const handlers = this.listeners.get(event);
      if (handlers) {
        for (const handler of handlers) {
          try {
            handler(payload);
          } catch (e) {
            dbgWarn("transport", "ws.handlerError", { event, error: e });
          }
        }
      }
    }
  }

  async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    await this.ensureConnected();

    const id = `req_${++this.reqId}`;
    dbg("transport", "ws.invoke", { cmd, id });

    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, {
        resolve: resolve as (v: unknown) => void,
        reject,
      });

      const message = JSON.stringify({
        id,
        method: cmd,
        params: args ?? {},
      });

      if (this.ws && this.ws.readyState === WebSocket.OPEN) {
        this.ws.send(message);
      } else {
        this.pending.delete(id);
        reject(new Error("WebSocket not connected"));
      }
    });
  }

  async listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
    dbg("transport", "ws.listen", { event });

    let handlers = this.listeners.get(event);
    if (!handlers) {
      handlers = new Set();
      this.listeners.set(event, handlers);
    }

    const typedHandler = handler as (payload: unknown) => void;
    handlers.add(typedHandler);

    // Ensure connection is established for receiving events
    this.ensureConnected().catch((e) => {
      dbgWarn("transport", "ws.listen.connectFailed", { event, error: e });
    });

    return () => {
      const h = this.listeners.get(event);
      if (h) {
        h.delete(typedHandler);
        if (h.size === 0) this.listeners.delete(event);
      }
    };
  }

  isDesktop(): boolean {
    return false;
  }
}
