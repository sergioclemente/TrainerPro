import {
  DEFAULT_VOICE_INTENT_THRESHOLD,
  type SemanticVoiceMatch,
  type VoiceSurface,
  type VoiceTranscriptPreparation,
} from "./voiceSurface";

export interface PreparedVoiceCommand<Context> {
  label: string;
  execute: (context: Context) => Promise<string>;
}

export interface VoiceCommandDefinition<Context> {
  phrases: readonly string[];
  available: (context: Context) => boolean;
  prepare: (
    match: SemanticVoiceMatch,
    context: Context,
  ) => PreparedVoiceCommand<Context> | null;
}

export type VoiceCommandDefinitions<Context> = Readonly<
  Record<string, VoiceCommandDefinition<Context>>
>;

/**
 * Preserves the literal command keys while checking every registry entry.
 * A screen defines each command once; the surface catalog and dispatch table
 * are derived from the resulting object.
 */
export function defineVoiceCommands<Context>() {
  return <const Definitions extends VoiceCommandDefinitions<Context>>(
    definitions: Definitions,
  ): Definitions => definitions;
}

interface CommandVoiceSurfaceOptions<
  Context,
  Definitions extends VoiceCommandDefinitions<Context>,
> {
  key: string;
  commands: Definitions;
  getContext: () => Context | null;
  contextUnavailableMessage: string;
  threshold?: number;
  prepareTranscript?: (
    transcript: string,
    context: Context,
  ) => VoiceTranscriptPreparation;
}

function definitionFor<Context>(
  definitions: VoiceCommandDefinitions<Context>,
  intentId: string,
): VoiceCommandDefinition<Context> | null {
  if (!Object.prototype.hasOwnProperty.call(definitions, intentId)) return null;
  return definitions[intentId] ?? null;
}

/** Builds the generic VoiceSurface contract from one screen-owned registry. */
export function createCommandVoiceSurface<
  Context,
  const Definitions extends VoiceCommandDefinitions<Context>,
>(options: CommandVoiceSurfaceOptions<Context, Definitions>): VoiceSurface {
  const catalog = Object.entries(options.commands).map(([id, definition]) => ({
    id,
    phrases: definition.phrases,
  }));

  return {
    key: options.key,
    catalog,
    threshold: options.threshold ?? DEFAULT_VOICE_INTENT_THRESHOLD,
    currentIntentGroups: () => {
      const context = options.getContext();
      if (!context) return [];
      return catalog.filter(({ id }) => options.commands[id].available(context));
    },
    prepareTranscript: options.prepareTranscript
      ? (transcript) => {
          const context = options.getContext();
          return context
            ? options.prepareTranscript!(transcript, context)
            : { kind: "rejected", visible: false };
        }
      : undefined,
    prepareCommand: (match) => {
      const context = options.getContext();
      const definition = definitionFor(options.commands, match.intentId);
      if (!context || !definition || !definition.available(context)) {
        return { kind: "rejected", visible: false };
      }

      const prepared = definition.prepare(match, context);
      if (!prepared) return { kind: "rejected", visible: true };

      return {
        kind: "command",
        label: prepared.label,
        execute: async () => {
          const liveContext = options.getContext();
          if (!liveContext) throw new Error(options.contextUnavailableMessage);
          return prepared.execute(liveContext);
        },
      };
    },
  };
}
