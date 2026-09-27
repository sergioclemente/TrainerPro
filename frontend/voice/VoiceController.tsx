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
import { traceEvent, type TraceFields } from "../trace";
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
  createVoiceMachine,
  reduceVoiceMachine,
  type VoiceMachineState,
  type VoiceMachineEvent,
} from "./voiceMachine";
import type { VoiceSurface } from "./voiceSurface";
import {
  type CaptureDiscontinuityReason,
  VOICE_AUDIO_CONSTRAINTS,
  WorkerMicTranscriber,
  type WorkerMicProgress,
} from "./workerMicTranscriber";

const VOICE_MAX_UTTERANCE_MS = 15_000;
const VOICE_NOTICE_DISPLAY_MS = 4_000;
const VOICE_TRANSCRIPT_DISPLAY_MS = 4_000;
const CAPTURE_MAX_AUTOMATIC_RESTARTS = 1;
const SEMANTIC_ROUTER_MAX_AUTOMATIC_RESTARTS = 1;
const COMMAND_REJECTED = "No matching command";
const ROUTER_RESTARTED = "Voice restarted — repeat the command";
const CAPTURE_DISCONTINUITY_MESSAGES: Record<CaptureDiscontinuityReason, string> = {
  "audio-interrupted": "Audio capture was interrupted",
  "devices-changed": "The available audio devices changed",
  "input-ended": "The microphone input ended",
  "input-muted": "The microphone input was interrupted",
};

export interface VoiceNotice {
  message: string;
  kind: "info" | "error";
}

export interface VoicePreparationProgress {
  source: "speech" | "commands";
  file: string;
  loadedBytes: number;
  totalBytes: number | null;
}

export type PushToTalkSource = "keyboard" | "controller";

interface VoiceContextValue {
  beginPushToTalk: (source: PushToTalkSource) => void;
  finishPushToTalk: (source: PushToTalkSource) => void;
  cancelPushToTalk: (source?: PushToTalkSource) => void;
  state: VoiceMachineState;
  progress: VoicePreparationProgress | null;
  notice: VoiceNotice | null;
  partialTranscript: string;
  finalTranscript: string;
  inputLevel: number;
  retry: () => void;
}

const VoiceContext = createContext<VoiceContextValue | null>(null);
const VoiceRegistrationContext = createContext<
  ((surface: VoiceSurface) => () => void) | null
>(null);

function errorMessage(error: unknown): string {
  return (error as AppError)?.message ?? (error instanceof Error ? error.message : String(error));
}

function isPermissionDenial(error: unknown): boolean {
  const name = (error as { name?: string })?.name;
  return name === "NotAllowedError" || name === "PermissionDeniedError";
}

function appIsActive(): boolean {
  return document.visibilityState === "visible";
}

function browserVoicePermission(state: PermissionState): VoiceMachineState["permission"] {
  return state === "prompt" ? "unknown" : state;
}

function traceVoiceRoute(
  utteranceId: number,
  generation: number,
  outcome: string,
  fields: TraceFields = {},
): void {
  traceEvent("voice_route", { utterance_id: utteranceId, generation, outcome, ...fields });
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
  const enabled = settings?.voice_enabled ?? false;

  const [state, rawDispatch] = useReducer(
    reduceVoiceMachine,
    undefined,
    () => createVoiceMachine({ enabled: false, appActive: appIsActive() }),
  );
  const [progress, setProgress] = useState<VoicePreparationProgress | null>(null);
  const [notice, setNotice] = useState<VoiceNotice | null>(null);
  const [partialTranscript, setPartialTranscript] = useState("");
  const [finalTranscript, setFinalTranscript] = useState("");
  const [inputLevel, setInputLevel] = useState(0);
  const [surfaceKey, setSurfaceKey] = useState<string | null>(null);

  const stateRef = useRef(state);
  stateRef.current = state;
  const dispatch = useCallback((event: VoiceMachineEvent) => {
    stateRef.current = reduceVoiceMachine(stateRef.current, event);
    rawDispatch(event);
  }, []);
  const pushToTalkRef = useRef<{ source: PushToTalkSource; generation: number; released: boolean } | null>(null);
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;
  const mountedRef = useRef(true);
  const permissionRequestRef = useRef<Promise<"granted" | "denied"> | null>(null);
  const transcriberRef = useRef<WorkerMicTranscriber | null>(null);
  const routerRef = useRef<SemanticRouterClient | null>(null);
  const routerRestartAttemptsRef = useRef(0);
  const modelUrlsRef = useRef<VoiceModelUrls | null>(null);
  const captureQueueRef = useRef<Promise<void>>(Promise.resolve());
  const captureGenerationRef = useRef(-1);
  const speechTimerRef = useRef<number | null>(null);
  const noticeTimerRef = useRef<number | null>(null);
  const transcriptTimerRef = useRef<number | null>(null);
  const surfaceRef = useRef<VoiceSurface | null>(null);
  const utteranceSequenceRef = useRef(0);
  const voiceOutageRef = useRef(false);

  const clearSpeechTimer = useCallback(() => {
    if (speechTimerRef.current !== null) window.clearTimeout(speechTimerRef.current);
    speechTimerRef.current = null;
  }, []);

  const clearNotice = useCallback(() => {
    if (noticeTimerRef.current !== null) window.clearTimeout(noticeTimerRef.current);
    noticeTimerRef.current = null;
    setNotice(null);
  }, []);

  const clearTranscripts = useCallback(() => {
    if (transcriptTimerRef.current !== null) {
      window.clearTimeout(transcriptTimerRef.current);
    }
    transcriptTimerRef.current = null;
    setPartialTranscript("");
    setFinalTranscript("");
  }, []);

  const scheduleTranscriptClear = useCallback(() => {
    if (transcriptTimerRef.current !== null) {
      window.clearTimeout(transcriptTimerRef.current);
    }
    transcriptTimerRef.current = window.setTimeout(() => {
      transcriptTimerRef.current = null;
      setFinalTranscript("");
    }, VOICE_TRANSCRIPT_DISPLAY_MS);
  }, []);

  const invalidateCapture = useCallback(() => {
    pushToTalkRef.current = null;
    transcriberRef.current?.invalidateCapture();
    captureGenerationRef.current = -1;
    clearSpeechTimer();
    clearTranscripts();
    setInputLevel(0);
  }, [clearSpeechTimer, clearTranscripts]);

  const registerSurface = useCallback((surface: VoiceSurface): (() => void) => {
    const current = surfaceRef.current;
    if (current && current !== surface) {
      throw new Error(`Voice surface ${current.key} is already active`);
    }
    surfaceRef.current = surface;
    invalidateCapture();
    clearNotice();
    stopVoiceFeedback();
    setSurfaceKey(surface.key);
    return () => {
      if (surfaceRef.current !== surface) return;
      surfaceRef.current = null;
      invalidateCapture();
      clearNotice();
      stopVoiceFeedback();
      setSurfaceKey(null);
    };
  }, [clearNotice, invalidateCapture]);

  const showNotice = useCallback((next: VoiceNotice) => {
    if (noticeTimerRef.current !== null) window.clearTimeout(noticeTimerRef.current);
    if (transcriptTimerRef.current !== null) {
      window.clearTimeout(transcriptTimerRef.current);
      transcriptTimerRef.current = null;
    }
    setNotice(next);
    noticeTimerRef.current = window.setTimeout(() => {
      noticeTimerRef.current = null;
      setNotice(null);
      setPartialTranscript("");
      setFinalTranscript("");
    }, VOICE_NOTICE_DISPLAY_MS);
  }, []);

  const enqueueCapture = useCallback(<T,>(operation: () => Promise<T>): Promise<T> => {
    const queued = captureQueueRef.current.catch(() => undefined).then(operation);
    captureQueueRef.current = queued.then(() => undefined, () => undefined);
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
    traceEvent("voice_error", {
      generation,
      code: (error as AppError)?.code ?? (error as { name?: string })?.name ?? "unknown",
    });
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
    traceEvent("voice_restart", {
      component: "semantic_router",
      attempt: routerRestartAttemptsRef.current,
    });
    showNotice({ message: ROUTER_RESTARTED, kind: "error" });
    void playVoiceErrorCue();
    dispatch({ type: "semantic-router-restart-requested", generation });
  }, [failCurrent, invalidateCapture, isCurrentCapture, showNotice]);

  const finishActivation = useCallback(async (generation: number): Promise<boolean> => {
    try {
      await enqueueCapture(async () => {
        if (!isCurrentCapture(generation)) return;
        await transcriberRef.current?.stop();
      });
      if (isCurrentCapture(generation)) pushToTalkRef.current = null;
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
      showNotice({ message: COMMAND_REJECTED, kind: "error" });
      await playVoiceErrorCue();
    } else {
      clearTranscripts();
    }
    if (await finishActivation(generation)) {
      dispatch({ type: "interpretation-rejected", generation });
    }
  }, [clearTranscripts, finishActivation, showNotice]);

  const routeCompletedLine = useCallback(async (
    transcript: string,
    generation: number,
    utteranceId: number,
    finalizedAt: number,
  ) => {
    const router = routerRef.current;
    const surface = surfaceRef.current;
    if (!router || !surface) {
      traceVoiceRoute(utteranceId, generation, "unavailable");
      await rejectUtterance(generation, false);
      return;
    }

    try {
      const transcriptPreparation = surface.prepareTranscript?.(transcript) ?? {
        kind: "route" as const,
        transcript,
      };
      if (transcriptPreparation.kind === "rejected") {
        traceVoiceRoute(utteranceId, generation, "transcript_rejected");
        await rejectUtterance(generation, transcriptPreparation.visible);
        return;
      }
      if (surface.currentIntentGroups().length === 0) {
        traceVoiceRoute(utteranceId, generation, "no_available_intent");
        await rejectUtterance(generation, false);
        return;
      }
      let result;
      try {
        result = await router.route(
          transcriptPreparation.transcript,
          surface.catalog,
          surface.threshold,
        );
        routerRestartAttemptsRef.current = 0;
      } catch (error) {
        traceVoiceRoute(
          utteranceId,
          generation,
          errorMessage(error).includes("timed out") ? "timeout" : "error",
        );
        recoverSemanticRouter(router, generation, error);
        return;
      }
      if (!isCurrentCapture(generation) || surfaceRef.current !== surface) {
        traceVoiceRoute(utteranceId, generation, "stale", {
          duration_ms: Math.round(result.totalDurationMs),
        });
        return;
      }
      if (result.intentId === null || result.matchedPhrase === null || result.score === null) {
        traceVoiceRoute(utteranceId, generation, "no_match", {
          score: result.score,
          duration_ms: Math.round(result.totalDurationMs),
        });
        await rejectUtterance(generation, true);
        return;
      }

      const match = {
        intentId: result.intentId,
        transcript: result.transcript,
        matchedPhrase: result.matchedPhrase,
        score: result.score,
      };
      const commandPreparation = surface.prepareCommand(match);
      if (commandPreparation.kind === "rejected") {
        traceVoiceRoute(utteranceId, generation, "command_rejected", {
          intent: result.intentId,
          score: result.score,
          duration_ms: Math.round(result.totalDurationMs),
        });
        await rejectUtterance(generation, commandPreparation.visible);
        return;
      }
      traceVoiceRoute(utteranceId, generation, "matched", {
        intent: result.intentId,
        score: result.score,
        duration_ms: Math.round(result.totalDurationMs),
      });
      dispatch({ type: "model-result", generation });
      const dispatchStartedAt = performance.now();
      try {
        await commandPreparation.execute();
        const current = isCurrentCapture(generation) && surfaceRef.current === surface;
        traceEvent("voice_dispatch", {
          utterance_id: utteranceId,
          generation,
          intent: result.intentId,
          outcome: current ? "ok" : "stale_after_execute",
          duration_ms: Math.round(performance.now() - dispatchStartedAt),
          line_to_action_ms: Math.round(performance.now() - finalizedAt),
        });
        if (!current) return;
        if (commandPreparation.successNotice) {
          showNotice({ message: commandPreparation.successNotice, kind: "info" });
        } else {
          scheduleTranscriptClear();
        }
        await playVoiceSuccessCue();
      } catch (error) {
        if (!isCurrentCapture(generation) || surfaceRef.current !== surface) return;
        traceEvent("voice_dispatch", {
          utterance_id: utteranceId,
          generation,
          intent: result.intentId,
          outcome: "error",
          error_code: (error as AppError)?.code ?? "unknown",
          duration_ms: Math.round(performance.now() - dispatchStartedAt),
          line_to_action_ms: Math.round(performance.now() - finalizedAt),
        });
        showNotice({ message: "Command failed", kind: "error" });
        await playVoiceErrorCue();
      }

      if (await finishActivation(generation)) {
        dispatch({ type: "command-finished", generation });
      }
    } catch (error) {
      failCurrent(generation, error);
    }
  }, [
    failCurrent,
    isCurrentCapture,
    recoverSemanticRouter,
    rejectUtterance,
    finishActivation,
    scheduleTranscriptClear,
    showNotice,
  ]);

  const cancelPushToTalk = useCallback((source?: PushToTalkSource) => {
    if (source && pushToTalkRef.current?.source !== source) return;
    invalidateCapture();
    dispatch({ type: "ptt-canceled" });
    void enqueueCapture(async () => { await transcriberRef.current?.stop(); }).catch(() => undefined);
  }, [invalidateCapture, enqueueCapture, dispatch]);

  const beginPushToTalk = useCallback((source: PushToTalkSource) => {
    if (!enabledRef.current || !surfaceRef.current || !appIsActive() ||
        (source === "keyboard" && !document.hasFocus()) ||
        stateRef.current.phase !== "idle" || pushToTalkRef.current) return;
    dispatch({ type: "ptt-pressed" });
    const generation = stateRef.current.generation;
    pushToTalkRef.current = { source, generation, released: false };
    clearNotice();
    clearTranscripts();
    speechTimerRef.current = window.setTimeout(() => {
      cancelPushToTalk(source);
      showNotice({ message: "Held too long — release and try again", kind: "error" });
    }, VOICE_MAX_UTTERANCE_MS);
    void enqueueCapture(async () => {
      if (!isCurrent(generation) || pushToTalkRef.current?.generation !== generation) return;
      captureGenerationRef.current = generation;
      await transcriberRef.current?.start();
      if (!isCurrentCapture(generation) || pushToTalkRef.current?.generation !== generation) {
        await transcriberRef.current?.stop();
        return;
      }
      dispatch({ type: "speech-started" });
      void playVoiceReadyCue();
    }).catch((error) => {
      if (!isCurrent(generation)) return;
      if (isPermissionDenial(error)) {
        invalidateCapture();
        dispatch({ type: "permission-changed", permission: "denied" });
      } else {
        failCurrent(generation, error);
      }
    });
  }, [dispatch, clearNotice, clearTranscripts, enqueueCapture, isCurrent, isCurrentCapture, cancelPushToTalk, showNotice, failCurrent, invalidateCapture]);

  const finishPushToTalk = useCallback((source: PushToTalkSource) => {
    const activation = pushToTalkRef.current;
    if (!activation || activation.source !== source || activation.released) return;
    activation.released = true;
    if (stateRef.current.phase === "starting") {
      cancelPushToTalk(source);
      return;
    }
    if (stateRef.current.phase !== "speech") return;
    clearSpeechTimer();
    transcriberRef.current?.mute(true);
    dispatch({ type: "ptt-released" });
    const generation = activation.generation;
    void enqueueCapture(async () => {
      if (!isCurrentCapture(generation)) return "";
      return (await transcriberRef.current?.finish()) ?? "";
    }).then((text) => {
      if (!isCurrentCapture(generation)) return;
      const transcript = text.trim();
      if (!transcript) {
        pushToTalkRef.current = null;
        dispatch({ type: "utterance-ignored" });
        return;
      }
      setPartialTranscript("");
      setFinalTranscript(transcript);
      dispatch({ type: "utterance-accepted" });
      const utteranceId = ++utteranceSequenceRef.current;
      traceEvent("voice_line_finalized", { utterance_id: utteranceId, generation });
      void routeCompletedLine(transcript, generation, utteranceId, performance.now());
    }).catch((error) => failCurrent(generation, error));
  }, [cancelPushToTalk, clearSpeechTimer, dispatch, enqueueCapture, isCurrentCapture, routeCompletedLine, failCurrent]);

  const recoverCapture = useCallback((
    generation: number,
    reason: CaptureDiscontinuityReason,
  ) => {
    if (!isCurrent(generation)) return;
    invalidateCapture();
    const reasonMessage = CAPTURE_DISCONTINUITY_MESSAGES[reason];
    traceEvent("voice_restart", {
      component: "capture",
      reason,
      attempt: stateRef.current.captureRestartAttempts + 1,
    });

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
    // Discard this hold immediately. Preparation warms models only;
    // another explicit press is required before reacquiring the microphone.
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
        if (mountedRef.current && isCurrentCapture(stateRef.current.generation)) setPartialTranscript(text.trim());
      },
      onLine: () => undefined, // Finalize the whole hold only after release.
      onAudioLevel: (level) => {
        if (mountedRef.current) setInputLevel(level);
      },
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
  }, [failCurrent, isCurrentCapture, recoverCapture]);

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
    if (!enabled || state.permission !== "unknown" || state.phase === "error") return;
    if (!navigator.mediaDevices?.getUserMedia) {
      failCurrent(stateRef.current.generation, new Error("Microphone capture is not supported"));
      return;
    }
    let canceled = false;
    const request = permissionRequestRef.current ??= (async () => {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: VOICE_AUDIO_CONSTRAINTS });
      stream.getTracks().forEach((track) => track.stop());
      return "granted" as const;
    })();
    void request.then((permission) => {
      if (permissionRequestRef.current === request) permissionRequestRef.current = null;
      if (!canceled && mountedRef.current && enabledRef.current) {
        dispatch({ type: "permission-changed", permission });
      }
    }).catch((error) => {
      if (permissionRequestRef.current === request) permissionRequestRef.current = null;
      if (canceled || !mountedRef.current || !enabledRef.current) return;
      if (isPermissionDenial(error)) {
        dispatch({ type: "permission-changed", permission: "denied" });
      } else {
        failCurrent(stateRef.current.generation, error);
      }
    });
    return () => { canceled = true; };
  }, [enabled, failCurrent, state.permission, state.phase]);

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
    if (!state.surfaceKey) {
      voiceOutageRef.current = false;
      return;
    }
    if (state.phase === "error" || state.phase === "unavailable") {
      if (voiceOutageRef.current) return;
      voiceOutageRef.current = true;
      useStore.getState().appendRideMessage(
        state.error ?? "Voice commands unavailable",
        "danger",
      );
      return;
    }
    if (state.phase === "idle" && voiceOutageRef.current) {
      voiceOutageRef.current = false;
      useStore.getState().appendRideMessage("Voice commands restored");
    }
  }, [state.error, state.phase, state.surfaceKey]);

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
    const blur = () => cancelPushToTalk("keyboard");
    window.addEventListener("blur", blur);
    window.addEventListener("pagehide", deactivate);
    window.addEventListener("pageshow", update);
    document.addEventListener("visibilitychange", update);
    document.addEventListener("freeze", deactivate);
    document.addEventListener("resume", update);
    return () => {
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", blur);
      window.removeEventListener("pagehide", deactivate);
      window.removeEventListener("pageshow", update);
      document.removeEventListener("visibilitychange", update);
      document.removeEventListener("freeze", deactivate);
      document.removeEventListener("resume", update);
    };
  }, [invalidateCapture, cancelPushToTalk]);

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
        if (!router) throw new Error("The semantic router is unavailable");
        await router.prepare(surface.catalog);
        if (canceled || !isCurrent(generation) || surfaceRef.current !== surface) return;
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
    clearNotice();
    stopVoiceFeedback();
    const transcriber = transcriberRef.current;
    captureGenerationRef.current = -1;
    if (state.phase === "suspended") {
      if (transcriber) void enqueueCapture(() => transcriber.stop()).catch(() => undefined);
    } else {
      disposeRuntimes();
    }
  }, [clearNotice, clearSpeechTimer, disposeRuntimes, enqueueCapture, state.phase]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      clearSpeechTimer();
      if (noticeTimerRef.current !== null) window.clearTimeout(noticeTimerRef.current);
      if (transcriptTimerRef.current !== null) window.clearTimeout(transcriptTimerRef.current);
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
    };
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
    <VoiceRegistrationContext.Provider value={registerSurface}>
      <VoiceContext.Provider value={{
        state,
        progress,
        notice,
        partialTranscript,
        finalTranscript,
        inputLevel,
        retry,
        beginPushToTalk, finishPushToTalk, cancelPushToTalk,
      }}>
        {children}
      </VoiceContext.Provider>
    </VoiceRegistrationContext.Provider>
  );
}

export function useVoice(): VoiceContextValue {
  const value = useContext(VoiceContext);
  if (!value) throw new Error("useVoice must be used inside VoiceProvider");
  return value;
}

/** Activates one screen-owned voice surface for the lifetime of that screen. */
export function useVoiceSurface(surface: VoiceSurface | null): void {
  const registerSurface = useContext(VoiceRegistrationContext);
  if (!registerSurface) throw new Error("useVoiceSurface must be used inside VoiceProvider");
  useEffect(() => {
    if (!surface) return;
    return registerSurface(surface);
  }, [registerSurface, surface]);
}
