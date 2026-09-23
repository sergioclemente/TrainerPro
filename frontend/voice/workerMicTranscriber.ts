import {
  ModelArch,
  SttWorkerHost,
  type TranscriptLine,
} from "@moonshine-ai/moonshine-wasm";
import voiceModelManifest from "./voice-models.manifest.json";
import moonshineSttWorkerUrl from "@moonshine-ai/moonshine-wasm/stt-worker?worker&url";
import moonshineWasmUrl from "@moonshine-ai/moonshine-wasm/moonshine.wasm?url";
import { traceEvent } from "../trace";

const TRANSCRIBER_ID = "trainerpro-voice";
const STREAM_ID = "trainerpro-voice-stream";
const SCRIPT_PROCESSOR_BUFFER_SIZE = 4096;
const AUDIO_LEVEL_UPDATE_INTERVAL_MS = 80;
const AUDIO_LEVEL_GAIN = 8;
// Short workout vocabulary biases decoding without a model retrain.
const MOONSHINE_KEYTERMS = "pause,resume,skip,intensity,ERG,decrease,increase,harder,easier,interval,segment,effort,ride,workout";
const MOONSHINE_KEYTERM_BOOST = 4.0;

export const VOICE_AUDIO_CONSTRAINTS = {
  // WKWebView echo cancellation suppresses the Camo virtual microphone to
  // near-silence. Request fan-noise suppression independently; keep echo
  // cancellation and automatic gain control disabled.
  echoCancellation: false,
  noiseSuppression: true,
  autoGainControl: false,
} satisfies MediaTrackConstraints;

export interface WorkerMicProgress {
  loaded: number;
  total: number | undefined;
  file: string;
}

export type CaptureDiscontinuityReason =
  | "audio-interrupted"
  | "devices-changed"
  | "input-ended"
  | "input-muted";

export interface WorkerMicCallbacks {
  onText: (text: string) => void;
  onLine: (line: TranscriptLine) => void;
  onError: (error: Error) => void;
  onCaptureDiscontinuity: (reason: CaptureDiscontinuityReason) => void;
  onProgress: (progress: WorkerMicProgress) => void;
  onAudioLevel?: (level: number) => void;
}

function downmixToMono(channels: readonly Float32Array[]): Float32Array {
  if (channels.length === 1) return new Float32Array(channels[0]);
  const frames = channels[0].length;
  const mixed = new Float32Array(frames);
  for (const channel of channels) {
    for (let frame = 0; frame < frames; frame += 1) mixed[frame] += channel[frame];
  }
  for (let frame = 0; frame < frames; frame += 1) mixed[frame] /= channels.length;
  return mixed;
}

const CAPTURE_WORKLET_SOURCE = `
const downmixToMono = ${downmixToMono.toString()};

class TrainerProVoiceCaptureProcessor extends AudioWorkletProcessor {
  process(inputs) {
    const input = inputs[0];
    if (input && input.length && input[0]) {
      const mono = downmixToMono(input);
      this.port.postMessage(mono, [mono.buffer]);
    }
    return true;
  }
}

registerProcessor("trainerpro-voice-capture", TrainerProVoiceCaptureProcessor);
`;

/**
 * System-default microphone capture backed by Moonshine's STT worker host.
 * The page owns only WebAudio lifecycle and transfers mono device-rate PCM;
 * Moonshine owns model state, resampling, VAD, and transcription off-thread.
 */
export class WorkerMicTranscriber {
  mediaStream?: MediaStream;
  audioContext?: AudioContext;
  sourceNode?: MediaStreamAudioSourceNode;
  workletNode?: AudioWorkletNode;
  scriptNode?: ScriptProcessorNode;

  private readonly host = new SttWorkerHost(
    new URL(moonshineWasmUrl, window.location.href).href,
    moonshineSttWorkerUrl,
  );
  private readonly callbacks: WorkerMicCallbacks;
  private readonly modelBaseUrl: string;
  private loaded = false;
  private loadPromise: Promise<void> | null = null;
  private running = false;
  private muted = false;
  private streamGeneration = 0;
  private captureGeneration = 0;
  private reportedDiscontinuityGeneration = -1;
  private deviceChangeListener?: () => void;
  private audioContextStateListener?: () => void;
  private lastAudioLevelAt = 0;
  private closed = false;

  constructor(modelBaseUrl: string, callbacks: WorkerMicCallbacks) {
    this.modelBaseUrl = modelBaseUrl;
    this.callbacks = callbacks;
    this.host.onProgress = (transcriberId, loaded, total, file) => {
      if (transcriberId === TRANSCRIBER_ID) callbacks.onProgress({ loaded, total, file });
    };
  }

  async load(): Promise<void> {
    if (this.loaded) return;
    if (this.closed) throw new Error("The microphone transcriber is closed");
    this.loadPromise ??= this.loadModel().finally(() => {
      this.loadPromise = null;
    });
    return this.loadPromise;
  }

  private async loadModel(): Promise<void> {
    const files = Object.fromEntries(
      voiceModelManifest.moonshine.files.map(({ path }) => [
        path,
        new URL(path, this.modelBaseUrl).href,
      ]),
    );
    await this.host.loadTranscriber({
      transcriberId: TRANSCRIBER_ID,
      modelArch: ModelArch.SmallStreaming,
      options: {
        keyterms: MOONSHINE_KEYTERMS,
        keyterm_boost: String(MOONSHINE_KEYTERM_BOOST),
      },
      source: { kind: "urls", files },
    });
    if (this.closed) throw new Error("The microphone transcriber closed while loading");
    this.loaded = true;
  }

  async start(): Promise<void> {
    if (this.running) return;
    if (this.closed) throw new Error("The microphone transcriber is closed");
    await this.load();
    this.running = true;
    try {
      await this.host.createStream(TRANSCRIBER_ID, STREAM_ID);
      this.setStreamListener();
      await this.host.start(STREAM_ID);
      this.mediaStream = await navigator.mediaDevices.getUserMedia({
        audio: VOICE_AUDIO_CONSTRAINTS,
      });
      const captureGeneration = ++this.captureGeneration;
      this.reportedDiscontinuityGeneration = -1;
      for (const track of this.mediaStream.getTracks()) {
        track.addEventListener("ended", () => {
          this.reportCaptureDiscontinuity(captureGeneration, "input-ended");
        }, { once: true });
        track.addEventListener("mute", () => {
          this.reportCaptureDiscontinuity(captureGeneration, "input-muted");
        }, { once: true });
      }
      this.audioContext = new AudioContext();
      if (this.audioContext.state === "suspended") await this.audioContext.resume();
      this.sourceNode = this.audioContext.createMediaStreamSource(this.mediaStream);

      const onAudio = (audio: Float32Array) => {
        if (!this.running || this.muted || !this.audioContext) return;
        this.reportAudioLevel(audio);
        this.host.addAudio(STREAM_ID, audio, this.audioContext.sampleRate);
      };
      if (this.audioContext.audioWorklet) {
        await this.setupWorklet(onAudio);
      } else {
        this.setupScriptProcessor(onAudio);
      }
      this.installContinuityListeners(captureGeneration);
      const settings = this.mediaStream.getAudioTracks()[0]?.getSettings();
      traceEvent("voice_capture_started", {
        noise_suppression_requested: VOICE_AUDIO_CONSTRAINTS.noiseSuppression,
        noise_suppression: settings?.noiseSuppression ?? "unknown",
        echo_cancellation: settings?.echoCancellation ?? "unknown",
        auto_gain_control: settings?.autoGainControl ?? "unknown",
      });
    } catch (error) {
      this.running = false;
      await this.releaseCapture();
      try {
        await this.host.closeStream(STREAM_ID);
      } catch {
        // The stream may not have been constructed yet.
      }
      const normalized = error instanceof Error ? error : new Error(String(error));
      throw normalized;
    }
  }

  mute(muted = true): void {
    this.muted = muted;
    if (muted) this.callbacks.onAudioLevel?.(0);
  }

  /** Immediately invalidate audio and any reset that could otherwise unmute it. */
  invalidateCapture(): void {
    this.muted = true;
    this.captureGeneration += 1;
    this.callbacks.onAudioLevel?.(0);
  }

  /** Discard streaming/VAD context without reacquiring the microphone. */
  async resetStream(): Promise<void> {
    if (!this.running) return;
    const captureGeneration = this.captureGeneration;
    this.muted = true;
    await this.host.stop(STREAM_ID);
    await this.host.closeStream(STREAM_ID);
    await this.host.createStream(TRANSCRIBER_ID, STREAM_ID);
    this.setStreamListener();
    await this.host.start(STREAM_ID);
    if (this.running && captureGeneration === this.captureGeneration) {
      this.muted = false;
    }
  }

  async stop(): Promise<void> {
    if (!this.running) return;
    this.running = false;
    this.streamGeneration += 1;
    this.captureGeneration += 1;
    await this.releaseCapture();
    try {
      await this.host.stop(STREAM_ID);
    } finally {
      await this.host.closeStream(STREAM_ID);
    }
  }

  close(): void {
    this.running = false;
    this.streamGeneration += 1;
    this.captureGeneration += 1;
    this.host.close();
    this.loaded = false;
    this.closed = true;
  }

  private setStreamListener(): void {
    const generation = ++this.streamGeneration;
    this.host.setListener(STREAM_ID, {
      onLineTextChanged: ({ line }) => {
        if (this.running && generation === this.streamGeneration) this.callbacks.onText(line.text);
      },
      onLineCompleted: ({ line }) => {
        if (this.running && generation === this.streamGeneration) this.callbacks.onLine(line);
      },
      onError: ({ error }) => {
        if (this.running && generation === this.streamGeneration) this.callbacks.onError(error);
      },
    });
  }

  private reportAudioLevel(audio: Float32Array): void {
    if (!this.callbacks.onAudioLevel) return;
    const now = performance.now();
    if (now - this.lastAudioLevelAt < AUDIO_LEVEL_UPDATE_INTERVAL_MS) return;
    this.lastAudioLevelAt = now;
    let sumOfSquares = 0;
    for (const sample of audio) sumOfSquares += sample * sample;
    const rms = audio.length > 0 ? Math.sqrt(sumOfSquares / audio.length) : 0;
    this.callbacks.onAudioLevel(Math.min(1, rms * AUDIO_LEVEL_GAIN));
  }

  private installContinuityListeners(captureGeneration: number): void {
    const context = this.audioContext;
    if (!context) throw new Error("Audio capture was not initialized");

    this.deviceChangeListener = () => {
      this.reportCaptureDiscontinuity(captureGeneration, "devices-changed");
    };
    navigator.mediaDevices.addEventListener("devicechange", this.deviceChangeListener);

    this.audioContextStateListener = () => {
      if (context.state !== "running") {
        this.reportCaptureDiscontinuity(captureGeneration, "audio-interrupted");
      }
    };
    context.addEventListener("statechange", this.audioContextStateListener);
  }

  private reportCaptureDiscontinuity(
    captureGeneration: number,
    reason: CaptureDiscontinuityReason,
  ): void {
    if (!this.running || captureGeneration !== this.captureGeneration ||
        this.reportedDiscontinuityGeneration === captureGeneration) {
      return;
    }
    this.reportedDiscontinuityGeneration = captureGeneration;
    this.muted = true;
    this.callbacks.onCaptureDiscontinuity(reason);
  }

  private async setupWorklet(onAudio: (audio: Float32Array) => void): Promise<void> {
    const context = this.audioContext;
    const source = this.sourceNode;
    if (!context || !source) throw new Error("Audio capture was not initialized");
    const moduleUrl = URL.createObjectURL(
      new Blob([CAPTURE_WORKLET_SOURCE], { type: "application/javascript" }),
    );
    try {
      await context.audioWorklet.addModule(moduleUrl);
    } finally {
      URL.revokeObjectURL(moduleUrl);
    }
    this.workletNode = new AudioWorkletNode(context, "trainerpro-voice-capture");
    this.workletNode.port.onmessage = (event: MessageEvent<Float32Array>) => onAudio(event.data);
    source.connect(this.workletNode);
    this.workletNode.connect(context.destination);
  }

  private setupScriptProcessor(onAudio: (audio: Float32Array) => void): void {
    const context = this.audioContext;
    const source = this.sourceNode;
    if (!context || !source) throw new Error("Audio capture was not initialized");
    this.scriptNode = context.createScriptProcessor(SCRIPT_PROCESSOR_BUFFER_SIZE, 1, 1);
    this.scriptNode.onaudioprocess = (event) => {
      onAudio(new Float32Array(event.inputBuffer.getChannelData(0)));
    };
    source.connect(this.scriptNode);
    this.scriptNode.connect(context.destination);
  }

  private async releaseCapture(): Promise<void> {
    this.callbacks.onAudioLevel?.(0);
    if (this.deviceChangeListener) {
      navigator.mediaDevices.removeEventListener("devicechange", this.deviceChangeListener);
      this.deviceChangeListener = undefined;
    }
    if (this.audioContext && this.audioContextStateListener) {
      this.audioContext.removeEventListener("statechange", this.audioContextStateListener);
      this.audioContextStateListener = undefined;
    }
    this.workletNode?.disconnect();
    this.scriptNode?.disconnect();
    this.sourceNode?.disconnect();
    this.mediaStream?.getTracks().forEach((track) => track.stop());
    await this.audioContext?.close();
    this.workletNode = undefined;
    this.scriptNode = undefined;
    this.sourceNode = undefined;
    this.mediaStream = undefined;
    this.audioContext = undefined;
  }
}
