import type { PlayerState } from "../ipc";
import {
  createCommandVoiceSurface,
  defineVoiceCommands,
} from "../voice/commandRegistry";
import { createIntegerSlotParser } from "../voice/spokenNumber";
import {
  normalizeVoiceTranscript,
  type VoiceSurface,
} from "../voice/voiceSurface";

export interface PlayerVoiceCommands {
  startRide: () => Promise<void>;
  pauseRide: () => Promise<void>;
  resumeRide: () => Promise<void>;
  skipSegment: () => Promise<void>;
  setIntensity: (intensity: number) => Promise<void>;
  setErg: (enabled: boolean) => Promise<void>;
}

export interface PlayerVoiceEnvironment {
  getPlayer: () => PlayerState | null;
  commands: PlayerVoiceCommands;
}

interface PlayerVoiceContext {
  player: PlayerState;
  commands: PlayerVoiceCommands;
}

const PERCENT_SCALE = 100;
const INTENSITY_PERCENT_MIN = 50;
const INTENSITY_PERCENT_MAX = 150;
const INTENSITY_PERCENT_STEP = 1;
const INTENSITY_ADJUSTMENT_MIN = 1;
const INTENSITY_ADJUSTMENT_MAX = 10;
const INTENSITY_ADJUSTMENT_STEP = 1;

const intensityPercent = createIntegerSlotParser({
  min: INTENSITY_PERCENT_MIN,
  max: INTENSITY_PERCENT_MAX,
  step: INTENSITY_PERCENT_STEP,
});
const intensityAdjustment = createIntegerSlotParser({
  min: INTENSITY_ADJUSTMENT_MIN,
  max: INTENSITY_ADJUSTMENT_MAX,
  step: INTENSITY_ADJUSTMENT_STEP,
});

const INCREASE_WORDS = /\b(?:increase|raise|higher|harder|up|add|bump)\b/;
const DECREASE_WORDS = /\b(?:decrease|reduce|lower|easier|softer|down|drop)\b/;
const END_RIDE_WORDS = /\b(?:end|finish)\b.*\b(?:ride|workout)\b|\bstop\b.*\b(?:ride|workout)\b/;
const END_RIDE_STT_SUBSTITUTION = /^and the ride\.?$/;
const END_RIDE_CANONICAL_PHRASE = "end the ride";

function phaseIs(...phases: readonly PlayerState["phase"][]) {
  return ({ player }: PlayerVoiceContext): boolean => phases.includes(player.phase);
}

function requireUnfinished(player: PlayerState): void {
  if (player.phase === "finished") throw new Error("The workout has finished");
}

function boundedIntensityPercent(percent: number): number {
  return Math.min(INTENSITY_PERCENT_MAX, Math.max(INTENSITY_PERCENT_MIN, percent));
}

function formatPercent(percent: number): string {
  return Number.isInteger(percent) ? String(percent) : percent.toFixed(1);
}

async function setIntensity(
  requestedPercent: number,
  { player, commands }: PlayerVoiceContext,
): Promise<string> {
  requireUnfinished(player);
  if (!Number.isFinite(requestedPercent)) throw new Error("Intensity must be a finite percentage");
  const percent = boundedIntensityPercent(requestedPercent);
  const intensity = percent / PERCENT_SCALE;
  if (intensity === player.intensity) return `Intensity already ${formatPercent(percent)}%`;
  await commands.setIntensity(intensity);
  return `Intensity set to ${formatPercent(percent)}%`;
}

const definePlayerCommands = defineVoiceCommands<PlayerVoiceContext>();

const PLAYER_COMMANDS = definePlayerCommands({
  start: {
    phrases: [
      "start the workout",
      "begin the ride",
      "start riding",
    ],
    available: phaseIs("ready"),
    prepare: () => ({
      label: "Start ride",
      execute: async ({ player, commands }) => {
        if (player.phase === "riding" || player.phase === "paused") return "Ride already started";
        if (player.phase !== "ready") throw new Error("The ride cannot be started now");
        await commands.startRide();
        return "Ride started";
      },
    }),
  },

  pause: {
    phrases: [
      "pause",
      "stop",
      "take a break",
      "hold the workout",
      "end the ride",
      "finish the workout",
    ],
    available: phaseIs("riding"),
    prepare: ({ transcript }) => {
      const endLike = END_RIDE_WORDS.test(normalizeVoiceTranscript(transcript));
      return {
        label: endLike ? "End ride" : "Pause workout",
        execute: async ({ player, commands }) => {
          if (player.phase === "paused") {
            return endLike ? "Workout paused — finish manually" : "Workout already paused";
          }
          if (player.phase !== "riding") throw new Error("The workout cannot be paused now");
          await commands.pauseRide();
          return endLike ? "Workout paused — finish manually" : "Workout paused";
        },
      };
    },
  },

  resume: {
    phrases: [
      "resume",
      "continue the ride",
      "keep going",
      "carry on",
      "unpause",
    ],
    available: phaseIs("paused"),
    prepare: () => ({
      label: "Resume workout",
      execute: async ({ player, commands }) => {
        if (player.phase === "riding") return "Workout already running";
        if (player.phase !== "paused") throw new Error("The workout cannot be resumed now");
        await commands.resumeRide();
        return "Workout resumed";
      },
    }),
  },

  skip: {
    phrases: [
      "skip",
      "next interval",
      "move to the next effort",
    ],
    available: phaseIs("riding", "paused"),
    prepare: () => ({
      label: "Skip interval",
      execute: async ({ player, commands }) => {
        requireUnfinished(player);
        if (player.phase === "ready") throw new Error("Start the ride before skipping an interval");
        await commands.skipSegment();
        return "Interval skipped";
      },
    }),
  },

  setIntensity: {
    phrases: [
      "set intensity to ninety five percent",
      "change the intensity to one twenty percent",
    ],
    available: phaseIs("ready", "riding", "paused"),
    prepare: ({ transcript }) => {
      const percent = intensityPercent.parse(transcript);
      return percent === null
        ? null
        : {
            label: `Set intensity to ${percent}%`,
            execute: (context) => setIntensity(percent, context),
          };
    },
  },

  adjustIntensity: {
    phrases: [
      "increase intensity by ten percent",
      "make it five percent harder",
      "decrease intensity by ten percent",
      "make it five percent easier",
    ],
    available: phaseIs("ready", "riding", "paused"),
    prepare: ({ transcript }) => {
      const normalized = normalizeVoiceTranscript(transcript);
      const increases = INCREASE_WORDS.test(normalized);
      const decreases = DECREASE_WORDS.test(normalized);
      const amount = intensityAdjustment.parse(normalized);
      if (amount === null || increases === decreases) return null;
      const delta = increases ? amount : -amount;
      const direction = delta > 0 ? "Increase" : "Decrease";
      return {
        label: `${direction} intensity by ${Math.abs(delta)}%`,
        execute: (context) => setIntensity(
          context.player.intensity * PERCENT_SCALE + delta,
          context,
        ),
      };
    },
  },

  ergOn: {
    phrases: ["enable erg mode", "erg mode on"],
    available: phaseIs("ready", "riding", "paused"),
    prepare: () => ({
      label: "Enable ERG",
      execute: async ({ player, commands }) => {
        requireUnfinished(player);
        if (player.erg_enabled) return "ERG already enabled";
        await commands.setErg(true);
        return "ERG enabled";
      },
    }),
  },

  ergOff: {
    phrases: ["disable erg mode", "erg mode off"],
    available: phaseIs("ready", "riding", "paused"),
    prepare: () => ({
      label: "Disable ERG",
      execute: async ({ player, commands }) => {
        requireUnfinished(player);
        if (!player.erg_enabled) return "ERG already disabled";
        await commands.setErg(false);
        return "ERG disabled";
      },
    }),
  },
});

export function createPlayerVoiceSurface(
  environment: PlayerVoiceEnvironment,
): VoiceSurface {
  return createCommandVoiceSurface({
    key: "player",
    commands: PLAYER_COMMANDS,
    getContext: () => {
      const player = environment.getPlayer();
      return player ? { player, commands: environment.commands } : null;
    },
    contextUnavailableMessage: "The workout is no longer active",
    prepareTranscript: (transcript, { player }) => {
      if (!END_RIDE_STT_SUBSTITUTION.test(normalizeVoiceTranscript(transcript))) {
        return { kind: "route", transcript };
      }
      return player.phase === "riding"
        ? { kind: "route", transcript: END_RIDE_CANONICAL_PHRASE }
        : { kind: "rejected", visible: false };
    },
  });
}
