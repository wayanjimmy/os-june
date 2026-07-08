export type HermesGatewayEventName =
  | "gateway.ready"
  | "session.info"
  | "message.start"
  | "message.delta"
  | "message.complete"
  | "thinking.delta"
  | "reasoning.delta"
  | "status.update"
  | "tool.start"
  | "tool.progress"
  | "tool.complete"
  | "clarify.request"
  | "clarify.response"
  | "approval.request"
  | "approval.response"
  | "subagent.start"
  | "subagent.tool"
  | "subagent.progress"
  | "subagent.thinking"
  | "subagent.complete"
  | "error"
  | (string & {});

export type HermesGatewayEvent<P = unknown> = {
  type: HermesGatewayEventName;
  session_id?: string;
  payload?: P;
};

type PendingCall = {
  method: string;
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
  timer?: number;
};

type Frame = {
  id?: string | number | null;
  method?: string;
  params?: HermesGatewayEvent;
  result?: unknown;
  error?: { code?: number; message?: string; method?: string };
};

export type HermesGatewayDiagnosticEvent = {
  event:
    | "ws.connect.started"
    | "ws.connect.open"
    | "ws.connect.error"
    | "ws.connect.timeout"
    | "ws.close"
    | "rpc.request.started"
    | "rpc.request.ok"
    | "rpc.request.error"
    | "rpc.request.timeout"
    | "event.received";
  at: string;
  method?: string;
  id?: string | number;
  url?: string;
  code?: number;
  message?: string;
  eventType?: string;
};

export type HermesGatewayDiagnosticSink = (event: HermesGatewayDiagnosticEvent) => void;

/** RPC rejection from the gateway, keeping the JSON-RPC error code so callers
 * can branch on well-known conditions instead of matching message strings. */
export class HermesGatewayError extends Error {
  readonly code?: number;

  constructor(message: string, code?: number) {
    super(message);
    this.name = "HermesGatewayError";
    this.code = code;
  }
}

// The gateway rejects prompt.submit (and other mutations) with 4009 while a
// turn is running — see tui_gateway/server.py `_err(rid, 4009, "session busy")`.
const SESSION_BUSY_CODE = 4009;

export function isSessionBusyError(err: unknown) {
  return err instanceof HermesGatewayError && err.code === SESSION_BUSY_CODE;
}

export class HermesGatewayClient {
  private nextId = 0;
  private pending = new Map<string | number, PendingCall>();
  private socket?: WebSocket;
  private connectPromise?: Promise<void>;
  private handlers = new Set<(event: HermesGatewayEvent) => void>();
  private closeHandlers = new Set<() => void>();

  constructor(private readonly diagnostics?: HermesGatewayDiagnosticSink) {}

  async connect(wsUrl: string) {
    if (this.socket?.readyState === WebSocket.OPEN) return;
    // Coalesce concurrent connects: a second caller arriving while the
    // handshake is in flight must not kill the first caller's socket — it
    // just awaits the same connection attempt.
    if (this.connectPromise) return this.connectPromise;
    this.close();
    this.recordDiagnostic({ event: "ws.connect.started", url: redactGatewayUrl(wsUrl) });
    const socket = new WebSocket(wsUrl);
    this.socket = socket;
    socket.addEventListener("message", (event) => this.handleMessage(event.data));
    socket.addEventListener("close", (event) => {
      // A stale socket's close event must not reject requests pending on
      // the socket that replaced it, nor notify close listeners.
      if (this.socket !== socket) return;
      this.socket = undefined;
      this.recordDiagnostic({
        event: "ws.close",
        code: closeEventCode(event),
        message: closeEventReason(event),
      });
      this.rejectAll(new Error("Hermes gateway connection closed."));
      for (const handler of [...this.closeHandlers]) handler();
    });
    const connectPromise = new Promise<void>((resolve, reject) => {
      const timer = window.setTimeout(() => {
        this.recordDiagnostic({ event: "ws.connect.timeout", url: redactGatewayUrl(wsUrl) });
        reject(new Error("Hermes gateway connection timed out."));
        socket.close();
      }, 15000);
      socket.addEventListener(
        "open",
        () => {
          window.clearTimeout(timer);
          this.recordDiagnostic({ event: "ws.connect.open", url: redactGatewayUrl(wsUrl) });
          resolve();
        },
        { once: true },
      );
      socket.addEventListener(
        "error",
        () => {
          window.clearTimeout(timer);
          this.recordDiagnostic({ event: "ws.connect.error", url: redactGatewayUrl(wsUrl) });
          reject(new Error("Could not connect to Hermes gateway."));
        },
        { once: true },
      );
    }).finally(() => {
      if (this.connectPromise === connectPromise) {
        this.connectPromise = undefined;
      }
    });
    this.connectPromise = connectPromise;
    return connectPromise;
  }

  close() {
    // Detach before closing so the close event reads as intentional — the
    // identity guard in the close handler then skips rejectAll/onClose.
    const socket = this.socket;
    this.socket = undefined;
    socket?.close();
  }

  onEvent(handler: (event: HermesGatewayEvent) => void) {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  // Fires only for unexpected disconnects of the current socket — an explicit
  // close() or a superseded (stale) socket does not notify.
  onClose(handler: () => void) {
    this.closeHandlers.add(handler);
    return () => {
      this.closeHandlers.delete(handler);
    };
  }

  request<T>(method: string, params: Record<string, unknown> = {}, timeoutMs = 120000) {
    const socket = this.socket;
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error("Hermes gateway is not connected."));
    }
    const id = ++this.nextId;
    this.recordDiagnostic({ event: "rpc.request.started", id, method });
    return new Promise<T>((resolve, reject) => {
      const pending: PendingCall = {
        method,
        resolve: (value) => resolve(value as T),
        reject,
      };
      pending.timer = window.setTimeout(() => {
        if (this.pending.delete(id)) {
          this.recordDiagnostic({ event: "rpc.request.timeout", id, method });
          reject(new Error(`Hermes request timed out: ${method}`));
        }
      }, timeoutMs);
      this.pending.set(id, pending);
      socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    });
  }

  private handleMessage(raw: unknown) {
    let frame: Frame;
    try {
      frame = JSON.parse(String(raw)) as Frame;
    } catch {
      return;
    }
    if (frame.id !== undefined && frame.id !== null) {
      const pending = this.pending.get(frame.id);
      if (!pending) return;
      if (pending.timer) window.clearTimeout(pending.timer);
      this.pending.delete(frame.id);
      if (frame.error) {
        this.recordDiagnostic({
          event: "rpc.request.error",
          id: frame.id,
          method: pending.method,
          code: frame.error.code,
          message: frame.error.message,
        });
        pending.reject(
          new HermesGatewayError(frame.error.message ?? "Hermes RPC failed.", frame.error.code),
        );
      } else {
        this.recordDiagnostic({ event: "rpc.request.ok", id: frame.id, method: pending.method });
        pending.resolve(frame.result);
      }
      return;
    }
    if (frame.method === "event" && frame.params?.type) {
      this.recordDiagnostic({ event: "event.received", eventType: frame.params.type });
      for (const handler of this.handlers) handler(frame.params);
    }
  }

  private recordDiagnostic(event: Omit<HermesGatewayDiagnosticEvent, "at">) {
    this.diagnostics?.({ ...event, at: new Date().toISOString() });
  }

  private rejectAll(error: Error) {
    for (const [id, pending] of this.pending) {
      if (pending.timer) window.clearTimeout(pending.timer);
      pending.reject(error);
      this.pending.delete(id);
    }
  }
}

function redactGatewayUrl(rawUrl: string) {
  try {
    const url = new URL(rawUrl);
    if (url.searchParams.has("token")) url.searchParams.set("token", "[redacted]");
    return url.toString();
  } catch {
    return rawUrl.replace(/([?&]token=)[^&]+/i, "$1[redacted]");
  }
}

function closeEventCode(event: unknown) {
  if (typeof event !== "object" || event === null || !("code" in event)) return undefined;
  const code = Number((event as { code?: unknown }).code);
  return Number.isFinite(code) ? code : undefined;
}

function closeEventReason(event: unknown) {
  return typeof event === "object" && event !== null && "reason" in event
    ? String((event as { reason?: unknown }).reason ?? "")
    : undefined;
}

export type HermesSessionCreateResponse = {
  session_id?: string;
  id?: string;
  [key: string]: unknown;
};
