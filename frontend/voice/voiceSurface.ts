export interface SemanticIntentGroup {
  id: string;
  phrases: readonly string[];
}

export interface SemanticVoiceMatch {
  intentId: string;
  transcript: string;
  matchedPhrase: string;
  score: number;
}

export type VoiceTranscriptPreparation =
  | { kind: "route"; transcript: string }
  | { kind: "rejected"; visible: boolean };

export type VoiceCommandPreparation =
  | {
      kind: "command";
      label: string;
      execute: () => Promise<string>;
    }
  | { kind: "rejected"; visible: boolean };

/**
 * A screen-owned declaration of the voice commands valid on that surface.
 * The shared voice runtime owns capture and routing; the surface owns meaning,
 * live validation, and execution.
 */
export interface VoiceSurface {
  /** Stable identity used to invalidate work when navigation changes context. */
  key: string;
  /** Full static catalog, prepared before capture starts. */
  catalog: readonly SemanticIntentGroup[];
  /** Context-filtered catalog evaluated immediately before each route. */
  currentIntentGroups: () => readonly SemanticIntentGroup[];
  threshold: number;
  prepareTranscript?: (transcript: string) => VoiceTranscriptPreparation;
  prepareCommand: (match: SemanticVoiceMatch) => VoiceCommandPreparation;
}

export const DEFAULT_VOICE_INTENT_THRESHOLD = 0.7;

export function normalizeVoiceTranscript(transcript: string): string {
  return transcript
    .normalize("NFKC")
    .toLowerCase()
    .replace(/[\u2010-\u2015-]+/g, " ")
    .replace(/[^a-z0-9.%\s]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}
