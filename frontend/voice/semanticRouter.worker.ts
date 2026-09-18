import {
  EmbeddingModel,
  EmbeddingModelArch,
} from "@moonshine-ai/moonshine-wasm";
import moonshineWasmUrl from "@moonshine-ai/moonshine-wasm/moonshine.wasm?url";
import voiceModelManifest from "./voice-models.manifest.json";
import type {
  SemanticRouterWorkerRequest,
  SemanticRouterWorkerResponse,
} from "./semanticRouterProtocol";
import { SemanticMatcher } from "./semanticMatcher";
import type { SemanticIntentGroup } from "./voiceSurface";

let model: EmbeddingModel | null = null;
let matcher: SemanticMatcher | null = null;
let busy = false;

function post(response: SemanticRouterWorkerResponse): void {
  self.postMessage(response);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function load(requestId: number, modelUrl: string): Promise<void> {
  if (busy) throw new Error("The semantic router worker is busy");
  busy = true;
  const startedAt = performance.now();
  try {
    model?.close();
    model = null;
    matcher = null;

    const files = Object.fromEntries(
      voiceModelManifest.embedding.files.map(({ path }) => [
        path,
        new URL(path, modelUrl).href,
      ]),
    );
    const exactWasmUrl = new URL(moonshineWasmUrl, self.location.href).href;
    const modelStartedAt = performance.now();
    model = await EmbeddingModel.loadFromUrls(files, {
      modelArch: EmbeddingModelArch.Gemma300M,
      variant: voiceModelManifest.embedding.variant,
      moduleOptions: {
        locateFile: (path) => path.endsWith(".wasm") ? exactWasmUrl : path,
      },
      onProgress: (loadedBytes, totalBytes, file) => {
        post({
          type: "progress",
          requestId,
          file,
          loadedBytes,
          totalBytes: totalBytes ?? null,
        });
      },
    });
    matcher = new SemanticMatcher(model);
    const modelLoadMs = performance.now() - modelStartedAt;

    post({
      type: "loaded",
      requestId,
      modelLoadMs,
      totalDurationMs: performance.now() - startedAt,
    });
  } catch (error) {
    model?.close();
    model = null;
    matcher = null;
    throw error;
  } finally {
    busy = false;
  }
}

function prepare(
  requestId: number,
  intentGroups: readonly SemanticIntentGroup[],
): void {
  if (busy) throw new Error("The semantic router worker is busy");
  if (!matcher) throw new Error("Load the semantic matcher before preparing a catalog");
  busy = true;
  const startedAt = performance.now();
  try {
    matcher.prepare(intentGroups);
    post({
      type: "prepared",
      requestId,
      warmupMs: performance.now() - startedAt,
      cachedPhraseCount: matcher.cachedPhraseCount,
    });
  } finally {
    busy = false;
  }
}

function route(
  requestId: number,
  transcript: string,
  intentGroups: readonly SemanticIntentGroup[],
  threshold: number,
): void {
  if (busy) throw new Error("The semantic router worker is busy");
  if (!matcher) throw new Error("Load the semantic matcher before routing");
  if (!Number.isFinite(threshold) || threshold < 0 || threshold > 1) {
    throw new Error("The semantic threshold must be between zero and one");
  }
  const trimmedTranscript = transcript.trim();
  if (!trimmedTranscript) throw new Error("Enter a transcript before routing");

  busy = true;
  const startedAt = performance.now();
  try {
    const { match, embeddingDurationMs } = matcher.bestMatch(trimmedTranscript, intentGroups);
    if (!match || match.score < threshold) {
      post({
        type: "result",
        requestId,
        transcript: trimmedTranscript,
        threshold,
        intentId: null,
        matchedPhrase: match?.phrase ?? null,
        score: match?.score ?? null,
        rejection: "no-match",
        embeddingDurationMs,
        totalDurationMs: performance.now() - startedAt,
      });
      return;
    }

    post({
      type: "result",
      requestId,
      transcript: trimmedTranscript,
      threshold,
      intentId: match.intentId,
      matchedPhrase: match.phrase,
      score: match.score,
      rejection: null,
      embeddingDurationMs,
      totalDurationMs: performance.now() - startedAt,
    });
  } finally {
    busy = false;
  }
}

function dispose(requestId: number): void {
  if (busy) throw new Error("The semantic router worker is busy");
  model?.close();
  model = null;
  matcher?.clear();
  matcher = null;
  post({ type: "disposed", requestId });
}

self.onmessage = async (event: MessageEvent<SemanticRouterWorkerRequest>) => {
  const request = event.data;
  try {
    if (request.type === "load") {
      await load(request.requestId, request.modelUrl);
    } else if (request.type === "prepare") {
      prepare(request.requestId, request.intentGroups);
    } else if (request.type === "route") {
      route(
        request.requestId,
        request.transcript,
        request.intentGroups,
        request.threshold,
      );
    } else if (request.type === "dispose") {
      dispose(request.requestId);
    } else {
      post({
        type: "error",
        requestId: 0,
        operation: "protocol",
        message: "Unknown semantic router worker request",
      });
    }
  } catch (error) {
    post({
      type: "error",
      requestId: request.requestId,
      operation: request.type,
      message: errorMessage(error),
    });
  }
};
