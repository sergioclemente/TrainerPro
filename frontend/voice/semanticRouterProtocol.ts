import type { SemanticIntentGroup } from "./voiceSurface";

export type SemanticRouterWorkerRequest =
  | {
      type: "load";
      requestId: number;
      modelUrl: string;
    }
  | {
      type: "prepare";
      requestId: number;
      intentGroups: readonly SemanticIntentGroup[];
    }
  | {
      type: "route";
      requestId: number;
      transcript: string;
      intentGroups: readonly SemanticIntentGroup[];
      threshold: number;
    }
  | {
      type: "dispose";
      requestId: number;
    };

export type SemanticRouterWorkerResponse =
  | {
      type: "progress";
      requestId: number;
      file: string;
      loadedBytes: number;
      totalBytes: number | null;
    }
  | {
      type: "loaded";
      requestId: number;
      modelLoadMs: number;
      totalDurationMs: number;
    }
  | {
      type: "prepared";
      requestId: number;
      warmupMs: number;
      cachedPhraseCount: number;
    }
  | {
      type: "result";
      requestId: number;
      transcript: string;
      threshold: number;
      intentId: string | null;
      matchedPhrase: string | null;
      score: number | null;
      rejection: "no-match" | null;
      embeddingDurationMs: number;
      totalDurationMs: number;
    }
  | {
      type: "disposed";
      requestId: number;
    }
  | {
      type: "error";
      requestId: number;
      operation: SemanticRouterWorkerRequest["type"] | "protocol";
      message: string;
    };
