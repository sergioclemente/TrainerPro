import type {
  SemanticRouterWorkerRequest,
  SemanticRouterWorkerResponse,
} from "./semanticRouterProtocol";
import type { SemanticIntentGroup } from "./voiceSurface";

type RouterResult = Extract<SemanticRouterWorkerResponse, { type: "result" }>;
type RouterProgress = Extract<SemanticRouterWorkerResponse, { type: "progress" }>;
type TerminalResponse = Exclude<SemanticRouterWorkerResponse, RouterProgress>;
type WithoutRequestId<T> = T extends { requestId: number } ? Omit<T, "requestId"> : never;
type WorkerRequestWithoutId = WithoutRequestId<SemanticRouterWorkerRequest>;

// Local model routing measured about 220 ms at p95 in the Node conformance
// harness. Two seconds leaves ample WebView/device headroom while still
// bounding a worker that has stopped responding.
export const SEMANTIC_ROUTE_TIMEOUT_MS = 2_000;

interface PendingRequest {
  requestId: number;
  expectedType: TerminalResponse["type"];
  resolve: (response: TerminalResponse) => void;
  reject: (error: Error) => void;
  timeoutId: ReturnType<typeof setTimeout> | null;
}

export interface SemanticRouterClientOptions {
  routeTimeoutMs?: number;
}

/** Owns the semantic worker and enforces its one-request, no-queue contract. */
export class SemanticRouterClient {
  private readonly worker: Worker;
  private readonly onProgress: (progress: RouterProgress) => void;
  private readonly routeTimeoutMs: number;
  private nextRequestId = 1;
  private pending: PendingRequest | null = null;
  private loaded = false;
  private loadPromise: Promise<void> | null = null;
  private closed = false;

  constructor(
    onProgress: (progress: RouterProgress) => void,
    options: SemanticRouterClientOptions = {},
  ) {
    this.onProgress = onProgress;
    this.routeTimeoutMs = options.routeTimeoutMs ?? SEMANTIC_ROUTE_TIMEOUT_MS;
    if (!Number.isFinite(this.routeTimeoutMs) || this.routeTimeoutMs <= 0) {
      throw new Error("The semantic route timeout must be a positive duration");
    }
    this.worker = new Worker(new URL("./semanticRouter.worker.ts", import.meta.url), {
      type: "module",
    });
    this.worker.onmessage = (event: MessageEvent<SemanticRouterWorkerResponse>) => {
      this.receive(event.data);
    };
    this.worker.onerror = (event) => {
      this.fail(new Error(event.message || "Semantic router worker crashed"));
    };
  }

  load(modelUrl: string): Promise<void> {
    if (this.loaded) return Promise.resolve();
    this.loadPromise ??= this.request({ type: "load", modelUrl }, "loaded").then(() => {
      this.loaded = true;
    }).finally(() => {
      this.loadPromise = null;
    });
    return this.loadPromise;
  }

  async route(
    transcript: string,
    intentGroups: readonly SemanticIntentGroup[],
    threshold: number,
  ): Promise<RouterResult> {
    if (!this.loaded) throw new Error("The semantic router is not loaded");
    return this.request(
      { type: "route", transcript, intentGroups, threshold },
      "result",
      this.routeTimeoutMs,
    ) as Promise<RouterResult>;
  }

  async prepare(intentGroups: readonly SemanticIntentGroup[]): Promise<void> {
    if (!this.loaded) throw new Error("The semantic router is not loaded");
    await this.request({ type: "prepare", intentGroups }, "prepared");
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.worker.terminate();
    this.rejectPending(new Error("Semantic router closed"));
    this.pending = null;
    this.loaded = false;
    this.loadPromise = null;
  }

  private request(
    request: WorkerRequestWithoutId,
    expectedType: TerminalResponse["type"],
    timeoutMs?: number,
  ): Promise<TerminalResponse> {
    if (this.closed) return Promise.reject(new Error("Semantic router closed"));
    if (this.pending) return Promise.reject(new Error("Semantic router worker is busy"));
    const requestId = this.nextRequestId++;
    return new Promise((resolve, reject) => {
      const timeoutId = timeoutMs === undefined
        ? null
        : globalThis.setTimeout(() => {
            if (this.pending?.requestId !== requestId) return;
            this.fail(new Error(`Semantic routing timed out after ${timeoutMs} ms`));
          }, timeoutMs);
      this.pending = { requestId, expectedType, resolve, reject, timeoutId };
      this.worker.postMessage({ ...request, requestId } satisfies SemanticRouterWorkerRequest);
    });
  }

  private receive(response: SemanticRouterWorkerResponse): void {
    if (response.type === "progress") {
      if (response.requestId === this.pending?.requestId) this.onProgress(response);
      return;
    }
    const pending = this.pending;
    if (!pending || response.requestId !== pending.requestId) return;
    this.pending = null;
    if (pending.timeoutId !== null) globalThis.clearTimeout(pending.timeoutId);
    if (response.type === "error") {
      pending.reject(new Error(response.message));
    } else if (response.type !== pending.expectedType) {
      pending.reject(new Error(`Unexpected semantic router response: ${response.type}`));
    } else {
      pending.resolve(response);
    }
  }

  private fail(error: Error): void {
    this.closed = true;
    this.loaded = false;
    this.loadPromise = null;
    this.worker.terminate();
    this.rejectPending(error);
  }

  private rejectPending(error: Error): void {
    const pending = this.pending;
    this.pending = null;
    if (!pending) return;
    if (pending.timeoutId !== null) globalThis.clearTimeout(pending.timeoutId);
    pending.reject(error);
  }
}
