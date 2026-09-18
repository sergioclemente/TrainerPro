import { createCommandVoiceSurface, defineVoiceCommands } from "./commandRegistry";
import type { VoiceSurface } from "./voiceSurface";

interface VoiceControlContext {
  commandsSuspended: boolean;
  setCommandsSuspended: (suspended: boolean) => void;
}

export interface VoiceControlEnvironment {
  getCommandsSuspended: () => boolean;
  setCommandsSuspended: (suspended: boolean) => void;
}

const defineVoiceControls = defineVoiceCommands<VoiceControlContext>();

const VOICE_CONTROLS = defineVoiceControls({
  suspendVoiceCommands: {
    phrases: [
      "stop listening",
      "stop talking",
      "pause voice commands",
      "mute voice commands",
    ],
    available: ({ commandsSuspended }) => !commandsSuspended,
    prepare: () => ({
      label: "Pause voice commands",
      execute: async ({ setCommandsSuspended }) => {
        setCommandsSuspended(true);
        return "Voice commands paused";
      },
    }),
  },

  resumeVoiceCommands: {
    phrases: [
      "resume listening",
      "start listening",
      "listen again",
      "resume voice commands",
    ],
    available: ({ commandsSuspended }) => commandsSuspended,
    prepare: () => ({
      label: "Resume voice commands",
      execute: async ({ setCommandsSuspended }) => {
        setCommandsSuspended(false);
        return "Voice commands active";
      },
    }),
  },
});

const VOICE_CONTROL_IDS = new Set(Object.keys(VOICE_CONTROLS));

export function isVoiceControlIntent(intentId: string): boolean {
  return VOICE_CONTROL_IDS.has(intentId);
}

export function createVoiceControlSurface(
  environment: VoiceControlEnvironment,
): VoiceSurface {
  return createCommandVoiceSurface({
    key: "voice-controls",
    commands: VOICE_CONTROLS,
    getContext: () => ({
      commandsSuspended: environment.getCommandsSuspended(),
      setCommandsSuspended: environment.setCommandsSuspended,
    }),
    contextUnavailableMessage: "Voice controls are unavailable",
  });
}
