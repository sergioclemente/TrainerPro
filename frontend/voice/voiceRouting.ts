import { isVoiceControlIntent } from "./voiceControls";
import type {
  SemanticIntentGroup,
  SemanticVoiceMatch,
  VoiceCommandPreparation,
  VoiceSurface,
} from "./voiceSurface";

export function voiceRoutingCatalog(
  surface: VoiceSurface,
  controls: VoiceSurface,
): readonly SemanticIntentGroup[] {
  return [...surface.catalog, ...controls.catalog];
}

export function currentVoiceIntentGroups(
  surface: VoiceSurface,
  controls: VoiceSurface,
  commandsSuspended: boolean,
): readonly SemanticIntentGroup[] {
  return [
    ...(commandsSuspended ? [] : surface.currentIntentGroups()),
    ...controls.currentIntentGroups(),
  ];
}

export function prepareRoutedVoiceCommand(
  surface: VoiceSurface,
  controls: VoiceSurface,
  commandsSuspended: boolean,
  match: SemanticVoiceMatch,
): VoiceCommandPreparation {
  if (isVoiceControlIntent(match.intentId)) return controls.prepareCommand(match);
  return commandsSuspended
    ? { kind: "rejected", visible: false }
    : surface.prepareCommand(match);
}
