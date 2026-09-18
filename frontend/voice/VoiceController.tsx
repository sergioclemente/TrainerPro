import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useReducer,
  useRef,
  useState,
} from "react";
import type { AppError } from "../ipc";
import {
  playVoiceErrorCue,
  playVoiceReadyCue,
  playVoiceSuccessCue,
  stopVoiceFeedback,
} from "../sounds";
import { useStore } from "../state";
import { resolveVoiceModelUrls, type VoiceModelUrls } from "./modelAssets";
import { SemanticRouterClient } from "./semanticRouterClient";
import {
  createVoiceControlSurface,
} from "./voiceControls";
import {
  currentVoiceIntentGroups,
  prepareRoutedVoiceCommand,
  voiceRoutingCatalog,
} from "./voiceRouting";
import {
  createVoiceMachine,
  reduceVoiceMachine,
  type VoiceMachineState,
} from "./voiceMachine";
import type { VoiceSurface } from "./voiceSurface";
import {
  type CaptureDiscontinuityReason,
  VOICE_AUDIO_CONSTRAINTS,
  WorkerMicTranscriber,
  type WorkerMicProgress,
} from "./workerMicTranscriber";

const VOICE_MAX_UTTERANCE_MS = 15_000;
const VOICE_RESULT_DISPLAY_MS = 4_000;
const CAPTURE_MAX_AUTOMATIC_RESTARTS = 1;
const SEMANTIC_ROUTER_MAX_AUTOMATIC_RESTARTS = 1;
const COMMAND_REJECTED = "Command not recognized";
const ROUTER_RESTARTED = "Voice restarted — repeat the command";
const CAPTURE_DISCONTINUITY_MESSAGES: Record<CaptureDiscontinuityReason, string> = {
  "audio-interrupted": "Audio capture was interrupted",
  "devices-changed": "The available audio devices changed",
  "input-ended": "The microphone input ended",
  "input-muted": "The microphone input was interrupted",
};

export interface VoiceFeedback {
  command: string;
  result: string;
  kind: "success" | "error";
}

export interface VoicePreparationProgress {
  source: "speech" | "commands";
  file: string;
  loadedBytes: number;
  totalBytes: number | null;
}

interface VoiceContextValue {
  state: VoiceMachineState;
  progress: VoicePreparationProgress | null;
  feedback: VoiceFeedback | null;
  retry: () => void;
  registerSurface: (surface: VoiceSurface) => () => void;
}

const VoiceContext = createContext<VoiceContextValue | null>(null);

function errorMessage(error: unknown): string {
  return (error as AppError)?.message ?? (error instanceof Error ? error.message : String(error));
}

function isPermissionDenial(error: unknown): boolean {
  const name = (error as { name?: string })?.name;
  return name === "NotAllowedError" || name === "PermissionDeniedError";
}

function appIsActive(): boolean {
  return document.visibilityState === "visible" && document.hasFocus();
}

function browserVoicePermission(state: PermissionState): VoiceMachineState["permission"] {
  return state === "prompt" ? "unknown" : state;
}

async function queryMicrophonePermission(): Promise<PermissionStatus | null> {
  if (!navigator.permissions?.query) return null;
  try {
    return await navigator.permissions.query({ name: "microphone" } as PermissionDescriptor);
  } catch {
    // WKWebView versions without microphone Permissions API support still use
    // getUserMedia and the capture track's ended event as authoritative paths.
    return null;
  }
}

export function VoiceProvider({ children }: { children: ReactNode }) {
  const settings = useStore((state) => state.settings);
  const pushToast = useStore((state) => state.pushToast);
  const enabled = settings?.voice_enabled ?? false;

  const [state, dispatch] = useReducer(
    reduceVoiceMachine,
    undefined,
    () => createVoiceMachine({ enabled: false, appActive: appIsActive() }),
  );
  const [progress, setProgress] = useState<VoicePreparationProgress | null>(null);
  const [feedback, setFeedback] = useState<VoiceFeedback | null>(null);
  const [surfaceKey, setSurfaceKey] = useState<string | null>(null);

  const stateRef = useRef(state);
  stateRef.current = state;
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;
  const mountedRef = useRef(true);
  const transcriberRef = useRef<WorkerMicTranscriber | null>(null);
  const routerRef = useRef<SemanticRouterClient | null>(null);
  const routerRestartAttemptsRef = useRef(0);
  const modelUrlsRef = useRef<VoiceModelUrls | null>(null);
  const captureQueueRef = useRef<Promise<void>>(Promise.resolve());
  const captureGenerationRef = useRef(-1);
  const speechTimerRef = useRef<number | null>(null);
  const feedbackTimerRef = useRef<number | null>(null);
  const surfaceRef = useRef<VoiceSurface | null>(null);
  const controlSurfaceRef = useRef<VoiceSurface | null>(null);
  controlSurfaceRef.current ??= createVoiceControlSurface({
    getCommandsSuspended: () => stateRef.current.commandsSuspended,
    setCommandsSuspended: (suspended) => {
      dispatch({ type: "command-suspension-changed", suspended });
    },
  });

  const clearSpeechTimer = useCallback(() => {
    if (speechTimerRef.current !== null) window.clearTimeout(speechTimerRef.current);
    speechTimerRef.current = null;
  }, []);

  const clearFeedback = useCallback(() => {
    if (feedbackTimerRef.current !== null) window.clearTimeout(feedbackTimerRef.current);
    feedbackTimerRef.current = null;
    setFeedback(null);
  }, []);

  const invalidateCapture = useCallback(() => {
    transcriberRef.current?.invalidateCapture();
    captureGenerationRef.current = -1;
    clearSpeechTimer();
  }, [clearSpeechTimer]);

  const registerSurface = useCallback((surface: VoiceSurface): (() => void) => {
    const current = surfaceRef.current;
    if (current && current !== surface) {
      throw new Error(`Voice surface ${current.key} is already active`);
    }
    surfaceRef.current = surface;
    invalidateCapture();
    clearFeedback();
    stopVoiceFeedback();
    setSurfaceKey(surface.key);
    return () => {
      if (surfaceRef.current !== surface) return;
      surfaceRef.current = null;
      invalidateCapture();
      clearFeedback();
      stopVoiceFeedback();
      setSurfaceKey(null);
    };
  }, [clearFeedback, invalidateCapture]);

  const showFeedback = useCallback((next: VoiceFeedback) => {
    if (feedbackTimerRef.current !== null) window.clearTimeout(feedbackTimerRef.current);
    setFeedback(next);
    feedbackTimerRef.current = window.setTimeout(() => {
      feedbackTimerRef.current = null;
      setFeedback(null);
    }, VOICE_RESULT_DISPLAY_MS);
  }, []);

  const enqueueCapture = useCallback((operation: () => Promise<void>): Promise<void> => {
    const queued = captureQueueRef.current.catch(() => undefined).then(operation);
    captureQueueRef.current = queued.catch(() => undefined);
    return queued;
  }, []);

  const isCurrent = useCallback((generation: number): boolean => {
    return mountedRef.current && stateRef.current.generation === generation;
  }, []);

  const isCurrentCapture = useCallback((generation: number): boolean => {
    return enabledRef.current && isCurrent(generation) &&
      captureGenerationRef.current === generation;
  }, [isCurrent]);

  const failCurrent = useCallback((generation: number, error: unknown) => {
    if (!isCurrent(generation)) return;
    invalidateCapture();
    void playVoiceErrorCue();
    dispatch({ type: "failed", generation, message: errorMessage(error) });
  }, [invalidateCapture, isCurrent]);

  const recoverSemanticRouter = useCallback((
    failedRouter: SemanticRouterClient,
    generation: number,
    error: unknown,
  ) => {
    if (routerRef.current === failedRouter) {
      routerRef.current = null;
      failedRouter.close();
    }
    if (!isCurrentCapture(generation)) return;

    invalidateCapture();
    if (routerRestartAttemptsRef.current >= SEMANTIC_ROUTER_MAX_AUTOMATIC_RESTARTS) {
      failCurrent(
        generation,
        new Error(`Semantic routing failed after restart: ${errorMessage(error)}`),
      );
      return;
    }

    routerRestartAttemptsRef.current += 1;
    showFeedback({
      command: "Voice command",
      result: ROUTER_RESTARTED,
      kind: "error",
    });
    void playVoiceErrorCue();
    dispatch({ type: "semantic-router-restart-requested", generation });
  }, [failCurrent, invalidateCapture, isCurrentCapture, showFeedback]);

  const resetStream = useCallback(async (generation: number): Promise<boolean> => {
    try {
      await enqueueCapture(async () => {
        if (!isCurrentCapture(generation)) return;
        await transcriberRef.current?.resetStream();
      });
      return isCurrentCapture(generation);
    } catch (error) {
      failCurrent(generation, error);
      return false;
    }
  }, [enqueueCapture, failCurrent, isCurrentCapture]);

  const rejectUtterance = useCallback(async (
    generation: number,
    visible: boolean,
  ) => {
    if (visible) {
      showFeedback({ command: "Voice command", result: COMMAND_REJECTED, kind: "error" });
      await playVoiceErrorCue();
    }
    if (await resetStream(generation)) {
      dispatch({ type: "interpretation-rejected", generation });
    }
  }, [resetStream, showFeedback]);

  const routeCompletedLine = useCallback(async (transcript: string, generation: number) => {
    const router = routerRef.current;
    const surface = surfaceRef.current;
    const controls = controlSurfaceRef.current;
    if (!router || !surface || !controls) {
      await rejectUtterance(generation, false);
      return;
    }

    try {
      const transcriptPreparation = surface.prepareTranscript?.(transcript) ?? {
        kind: "route" as const,
        transcript,
      };
      if (transcriptPreparation.kind === "rejected") {
        await rejectUtterance(generation, transcriptPreparation.visible);
        return;
      }
      const intentGroups = currentVoiceIntentGroups(
        surface,
        controls,
        stateRef.current.commandsSuspended,
      );
      if (intentGroups.length === 0) {
        await rejectUtterance(generation, false);
        return;
      }
      let result;
      try {
        result = await router.route(
          transcriptPreparation.transcript,
          voiceRoutingCatalog(surface, controls),
          surface.threshold,
        );
        routerRestartAttemptsRef.current = 0;
      } catch (error) {
        recoverSemanticRouter(router, generation, error);
        return;
      }
      if (!isCurrentCapture(generation) || surfaceRef.current !== surface) return;
      if (result.intentId === null || result.matchedPhrase === null || result.score === null) {
        await rejectUtterance(generation, false);
        return;
      }

      const match = {
        intentId: result.intentId,
        transcript: result.transcript,
        matchedPhrase: result.matchedPhrase,
        score: result.score,
      };
      const commandPreparation = prepareRoutedVoiceCommand(
        surface,
        controls,
        stateRef.current.commandsSuspended,
        match,
      );
      if (commandPreparation.kind === "rejected") {
        await rejectUtterance(generation, commandPreparation.visible);
        return;
      }
      dispatch({ type: "model-result", generation });
      try {
        const outcome = await commandPreparation.execute();
        if (!isCurrentCapture(generation) || surfaceRef.current !== surface) return;
        showFeedback({ command: commandPreparation.label, result: outcome, kind: "success" });
        await playVoiceSuccessCue();
      } catch (error) {
        if (!isCurrentCapture(generation) || surfaceRef.current !== surface) return;
        const message = errorMessage(error);
        pushToast("error", message);
        showFeedback({
          command: commandPreparation.label,
          result: "Command failed",
          kind: "error",
        });
        await playVoiceErrorCue();
      }

      if (await resetStream(generation)) {
        dispatch({ type: "command-finished", generation });
      }
    } catch (error) {
      failCurrent(generation, error);
    }
  }, [
    failCurrent,
    isCurrentCapture,
    pushToast,
    recoverSemanticRouter,
    rejectUtterance,
    resetStream,
    showFeedback,
  ]);

  const finishLongUtterance = useCallback(async (generation: number) => {
    if (!isCurrentCapture(generation) || stateRef.current.phase !== "speech") return;
    transcriberRef.current?.mute(true);
    showFeedback({ command: "Voice command", result: COMMAND_REJECTED, kind: "error" });
    await playVoiceErrorCue();
    if (await resetStream(generation)) dispatch({ type: "utterance-ignored" });
  }, [isCurrentCapture, resetStream, showFeedback]);

  const noteSpeech = useCallback(() => {
    const current = stateRef.current;
    if (captureGenerationRef.current !== current.generation || !surfaceRef.current) return;
    if (current.phase !== "listening" && current.phase !== "speech") return;
    if (current.phase === "listening") dispatch({ type: "speech-started" });
    if (speechTimerRef.current !== null) return;
    const generation = current.generation;
    speechTimerRef.current = window.setTimeout(() => {
      speechTimerRef.current = null;
      void finishLongUtterance(generation);
    }, VOICE_MAX_UTTERANCE_MS);
  }, [finishLongUtterance]);

  const acceptLine = useCallback((text: string) => {
    const current = stateRef.current;
    if (captureGenerationRef.current !== current.generation || !surfaceRef.current) return;
    if (current.phase !== "listening" && current.phase !== "speech") return;
    clearSpeechTimer();
    transcriberRef.current?.mute(true);
    dispatch({ type: "speech-started" });
    const transcript = text.trim();
    if (!transcript) {
      void resetStream(current.generation).then((fresh) => {
        if (fresh) dispatch({ type: "utterance-ignored" });
      });
      return;
    }
    dispatch({ type: "utterance-accepted" });
    void routeCompletedLine(transcript, current.generation);
  }, [clearSpeechTimer, resetStream, routeCompletedLine]);

  const recoverCapture = useCallback((
    generation: number,
    reason: CaptureDiscontinuityReason,
  ) => {
    if (!isCurrent(generation)) return;
    invalidateCapture();
    const reasonMessage = CAPTURE_DISCONTINUITY_MESSAGES[reason];

    void queryMicrophonePermission().then((permission) => {
      if (permission?.state === "denied") {
        dispatch({ type: "permission-changed", permission: "denied" });
      }
    });

    if (stateRef.current.captureRestartAttempts >= CAPTURE_MAX_AUTOMATIC_RESTARTS) {
      failCurrent(
        generation,
        new Error(`${reasonMessage} again after automatic reconnection`),
      );
      return;
    }

    const restartGeneration = generation + 1;
    const transcriber = transcriberRef.current;
    const stopping = enqueueCapture(async () => {
      await transcriber?.stop();
    });
    // Invalidate routing immediately. The preparing effect queues the new
    // capture after this stop, so audio from either input cannot be joined.
    dispatch({ type: "capture-restart-requested", generation });
    void stopping.catch((error) => {
      failCurrent(
        restartGeneration,
        new Error(`${reasonMessage}; recovery failed: ${errorMessage(error)}`),
      );
    });
  }, [enqueueCapture, failCurrent, invalidateCapture, isCurrent]);

  const ensureRuntimes = useCallback(async (urls: VoiceModelUrls) => {
    routerRef.current ??= new SemanticRouterClient((update) => {
      if (!mountedRef.current) return;
      setProgress({
        source: "commands",
        file: update.file,
        loadedBytes: update.loadedBytes,
        totalBytes: update.totalBytes,
      });
    });
    transcriberRef.current ??= new WorkerMicTranscriber(urls.moonshine, {
      onText: (text) => {
        if (text.trim()) noteSpeech();
      },
      onLine: (line) => acceptLine(line.text),
      onError: (error) => failCurrent(captureGenerationRef.current, error),
      onCaptureDiscontinuity: (reason) => {
        recoverCapture(captureGenerationRef.current, reason);
      },
      onProgress: (update: WorkerMicProgress) => {
        if (!mountedRef.current) return;
        setProgress({
          source: "speech",
          file: update.file,
          loadedBytes: update.loaded,
          totalBytes: update.total ?? null,
        });
      },
    });
    await Promise.all([
      routerRef.current.load(urls.embedding).catch((error) => {
        throw new Error(`Command model failed to load: ${errorMessage(error)}`);
      }),
      transcriberRef.current.load().catch((error) => {
        throw new Error(`Speech model failed to load: ${errorMessage(error)}`);
      }),
    ]);
  }, [acceptLine, failCurrent, noteSpeech, recoverCapture]);

  const disposeRuntimes = useCallback(() => {
    const router = routerRef.current;
    routerRef.current = null;
    router?.close();
    const transcriber = transcriberRef.current;
    transcriberRef.current = null;
    captureGenerationRef.current = -1;
    if (transcriber) {
      void enqueueCapture(async () => {
        await transcriber.stop();
        transcriber.close();
      }).catch(() => transcriber.close());
    }
    modelUrlsRef.current = null;
    setProgress(null);
  }, [enqueueCapture]);

  useEffect(() => {
    if (!enabled) {
      routerRestartAttemptsRef.current = 0;
      invalidateCapture();
    }
    dispatch({ type: "enabled-changed", enabled });
  }, [enabled, invalidateCapture]);

  useEffect(() => {
    if (!enabled || state.permission === "unknown") return;
    let canceled = false;
    let permissionStatus: PermissionStatus | null = null;
    const update = () => {
      if (!canceled && permissionStatus) {
        const permission = browserVoicePermission(permissionStatus.state);
        if (permission !== "granted") invalidateCapture();
        dispatch({
          type: "permission-changed",
          permission,
        });
      }
    };
    void queryMicrophonePermission().then((status) => {
      if (canceled || !status) return;
      permissionStatus = status;
      if (status.state === "denied") update();
      status.addEventListener("change", update);
    });
    return () => {
      canceled = true;
      permissionStatus?.removeEventListener("change", update);
    };
  }, [enabled, invalidateCapture, state.permission]);

  useEffect(() => {
    dispatch({ type: "surface-changed", key: surfaceKey });
  }, [surfaceKey]);

  useEffect(() => {
    const update = () => {
      const active = appIsActive();
      if (!active) invalidateCapture();
      dispatch({ type: "app-activity-changed", active });
    };
    const deactivate = () => {
      invalidateCapture();
      dispatch({ type: "app-activity-changed", active: false });
    };
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    window.addEventListener("pagehide", deactivate);
    window.addEventListener("pageshow", update);
    document.addEventListener("visibilitychange", update);
    document.addEventListener("freeze", deactivate);
    document.addEventListener("resume", update);
    return () => {
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
      window.removeEventListener("pagehide", deactivate);
      window.removeEventListener("pageshow", update);
      document.removeEventListener("visibilitychange", update);
      document.removeEventListener("freeze", deactivate);
      document.removeEventListener("resume", update);
    };
  }, [invalidateCapture]);

  useEffect(() => {
    if (state.phase !== "permission-required") return;
    const generation = state.generation;
    if (!navigator.mediaDevices?.getUserMedia) {
      failCurrent(generation, new Error("Microphone capture is not supported"));
      return;
    }
    let canceled = false;
    void navigator.mediaDevices.getUserMedia({ audio: VOICE_AUDIO_CONSTRAINTS }).then((stream) => {
      stream.getTracks().forEach((track) => track.stop());
      if (!canceled && isCurrent(generation)) {
        dispatch({ type: "permission-changed", permission: "granted" });
      }
    }).catch((error) => {
      if (canceled || !isCurrent(generation)) return;
      if (isPermissionDenial(error)) {
        invalidateCapture();
        dispatch({ type: "permission-changed", permission: "denied" });
      } else {
        failCurrent(generation, error);
      }
    });
    return () => {
      canceled = true;
    };
  }, [failCurrent, invalidateCapture, isCurrent, state.generation, state.phase]);

  useEffect(() => {
    if (state.phase !== "preparing") return;
    const generation = state.generation;
    let canceled = false;
    setProgress(null);
    void (async () => {
      try {
        const surface = surfaceRef.current;
        if (!surface) return;
        modelUrlsRef.current ??= await resolveVoiceModelUrls();
        await ensureRuntimes(modelUrlsRef.current);
        if (canceled || !isCurrent(generation) || surfaceRef.current !== surface) return;
        const router = routerRef.current;
        const controls = controlSurfaceRef.current;
        if (!router || !controls) throw new Error("The semantic router is unavailable");
        await router.prepare(voiceRoutingCatalog(surface, controls));
        if (canceled || !isCurrent(generation) || surfaceRef.current !== surface) return;
        await enqueueCapture(async () => {
          if (canceled || !isCurrent(generation) || surfaceRef.current !== surface) return;
          captureGenerationRef.current = generation;
          try {
            await transcriberRef.current?.start();
            transcriberRef.current?.mute(true);
            await playVoiceReadyCue();
            if (canceled || !isCurrent(generation) || surfaceRef.current !== surface) return;
            await transcriberRef.current?.resetStream();
          } catch (error) {
            if (isPermissionDenial(error) || stateRef.current.captureRestartAttempts === 0) {
              throw error;
            }
            throw new Error(`The microphone could not be reconnected: ${errorMessage(error)}`);
          }
        });
        if (!canceled && isCurrent(generation) && surfaceRef.current === surface) {
          setProgress(null);
          dispatch({ type: "prepared", generation });
        }
      } catch (error) {
        if (canceled || !isCurrent(generation)) return;
        if (isPermissionDenial(error)) {
          invalidateCapture();
          dispatch({ type: "permission-changed", permission: "denied" });
        } else {
          failCurrent(generation, error);
        }
      }
    })();
    return () => {
      canceled = true;
    };
  }, [
    enqueueCapture,
    ensureRuntimes,
    failCurrent,
    invalidateCapture,
    isCurrent,
    state.generation,
    state.phase,
  ]);

  useEffect(() => {
    if (state.phase !== "suspended" && state.phase !== "off" &&
        state.phase !== "permission-required" &&
        state.phase !== "unavailable" && state.phase !== "error") return;
    clearSpeechTimer();
    clearFeedback();
    stopVoiceFeedback();
    const transcriber = transcriberRef.current;
    captureGenerationRef.current = -1;
    if (state.phase === "suspended") {
      if (transcriber) void enqueueCapture(() => transcriber.stop()).catch(() => undefined);
    } else {
      disposeRuntimes();
    }
  }, [clearFeedback, clearSpeechTimer, disposeRuntimes, enqueueCapture, state.phase]);

  useEffect(() => () => {
    mountedRef.current = false;
    clearSpeechTimer();
    if (feedbackTimerRef.current !== null) window.clearTimeout(feedbackTimerRef.current);
    routerRef.current?.close();
    const transcriber = transcriberRef.current;
    transcriberRef.current = null;
    captureGenerationRef.current = -1;
    if (transcriber) {
      void enqueueCapture(async () => {
        await transcriber.stop();
        transcriber.close();
      }).catch(() => transcriber.close());
    }
  }, [clearSpeechTimer, enqueueCapture]);

  const retry = useCallback(() => {
    routerRestartAttemptsRef.current = 0;
    if (stateRef.current.permission === "denied") {
      dispatch({ type: "permission-changed", permission: "unknown" });
    } else {
      dispatch({ type: "retry" });
    }
  }, []);

  return (
    <VoiceContext.Provider value={{ state, progress, feedback, retry, registerSurface }}>
      {children}
    </VoiceContext.Provider>
  );
}

export function useVoice(): VoiceContextValue {
  const value = useContext(VoiceContext);
  if (!value) throw new Error("useVoice must be used inside VoiceProvider");
  return value;
}

/** Activates one screen-owned voice surface for the lifetime of that screen. */
export function useVoiceSurface(surface: VoiceSurface | null): void {
  const context = useContext(VoiceContext);
  if (!context) throw new Error("useVoiceSurface must be used inside VoiceProvider");
  const { registerSurface } = context;
  useEffect(() => {
    if (!surface) return;
    return registerSurface(surface);
  }, [registerSurface, surface]);
}
