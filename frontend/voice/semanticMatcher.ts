import { normalizeVoiceTranscript, type SemanticIntentGroup } from "./voiceSurface";

export interface SemanticEmbeddingModel {
  calculateEmbedding: (sentence: string) => Float32Array;
  distance: (embeddingA: Float32Array, embeddingB: Float32Array) => number;
}

export interface SemanticBestMatch {
  intentId: string;
  phrase: string;
  score: number;
}

export interface SemanticMatchResult {
  match: SemanticBestMatch | null;
  embeddingDurationMs: number;
}

/** Shared production/conformance matcher over one loaded embedding model. */
export class SemanticMatcher {
  private readonly phraseEmbeddings = new Map<string, Float32Array>();

  constructor(private readonly model: SemanticEmbeddingModel) {}

  get cachedPhraseCount(): number {
    return this.phraseEmbeddings.size;
  }

  prepare(intentGroups: readonly SemanticIntentGroup[]): void {
    for (const phrase of new Set(intentGroups.flatMap((group) => group.phrases))) {
      this.embeddingFor(phrase);
    }
  }

  bestMatch(
    transcript: string,
    intentGroups: readonly SemanticIntentGroup[],
  ): SemanticMatchResult {
    const embeddingStartedAt = performance.now();
    const utteranceEmbedding = this.model.calculateEmbedding(
      normalizeVoiceTranscript(transcript),
    );
    const embeddingDurationMs = performance.now() - embeddingStartedAt;
    let match: SemanticBestMatch | null = null;
    for (const group of intentGroups) {
      for (const phrase of group.phrases) {
        const score = this.model.distance(utteranceEmbedding, this.embeddingFor(phrase));
        if (!match || score > match.score) {
          match = { intentId: group.id, phrase, score };
        }
      }
    }
    return { match, embeddingDurationMs };
  }

  clear(): void {
    this.phraseEmbeddings.clear();
  }

  private embeddingFor(phrase: string): Float32Array {
    const cached = this.phraseEmbeddings.get(phrase);
    if (cached) return cached;
    const embedding = this.model.calculateEmbedding(phrase);
    this.phraseEmbeddings.set(phrase, embedding);
    return embedding;
  }
}
