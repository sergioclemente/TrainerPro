// Player audio feedback. Workout interval cues use the two clips inherited
// from WorkoutPlanner (public/countdown.wav and public/end.wav); voice-command
// readiness and outcomes use short synthesized tones below.

/** How far ahead of a segment boundary the countdown clip starts. */
export const COUNTDOWN_LEAD_S = 3.5;

const MUTE_KEY = "trainerpro.sounds.muted";
const VOICE_CUE_GAIN = 0.08;
const VOICE_CUE_ATTACK_S = 0.008;
const VOICE_CUE_RELEASE_S = 0.025;
const VOICE_CUE_START_DELAY_S = 0.01;
const VOICE_CUE_COMPLETION_GRACE_MS = 25;

interface VoiceCueNote {
  frequencyHz: number;
  offsetS: number;
  durationS: number;
}

const VOICE_READY_CUE: readonly VoiceCueNote[] = [
  { frequencyHz: 523, offsetS: 0, durationS: 0.08 },
];
const VOICE_SUCCESS_CUE: readonly VoiceCueNote[] = [
  { frequencyHz: 523, offsetS: 0, durationS: 0.07 },
  { frequencyHz: 659, offsetS: 0.075, durationS: 0.09 },
];
const VOICE_ERROR_CUE: readonly VoiceCueNote[] = [
  { frequencyHz: 294, offsetS: 0, durationS: 0.08 },
  { frequencyHz: 220, offsetS: 0.085, durationS: 0.11 },
];

let muted = localStorage.getItem(MUTE_KEY) === "1";
let countdown: HTMLAudioElement | null = null;
let end: HTMLAudioElement | null = null;
let primed = false;
let voiceContext: AudioContext | null = null;
const activeVoiceSources = new Set<OscillatorNode>();

function clips(): HTMLAudioElement[] {
  if (!countdown) {
    countdown = new Audio("/countdown.wav");
    countdown.preload = "auto";
    end = new Audio("/end.wav");
    end.preload = "auto";
  }
  return [countdown, end!];
}

/**
 * WKWebView only allows playback that descends from a user gesture, and the
 * cues fire from a timer — so each element must be play()ed once inside a real
 * gesture to unlock it for the rest of the session.
 *
 * MUST be called only from a ride-start gesture (Start/Resume), never from a
 * global listener: an earlier version primed on the first click anywhere in the
 * app, which played a 3.5 s countdown at someone browsing the Library.
 *
 * Silence here is structural, not best-effort. `pause()` runs in the SAME TICK
 * as `play()`, so playback is cancelled before a frame of audio is emitted —
 * the previous version paused inside `.then()`, which only fires once playback
 * has already begun, and `muted` set moments earlier is not reliably applied to
 * a not-yet-loaded element. volume/muted are belt-and-braces on top.
 */
export function primeSounds() {
  if (primed) return;
  primed = true;
  for (const a of clips()) {
    a.muted = true;
    a.volume = 0;
    const p = a.play();
    a.pause();
    a.currentTime = 0;
    // play() rejects with AbortError because we paused it — that is the
    // intended path, and the element is unlocked either way.
    void Promise.resolve(p)
      .catch(() => {})
      .then(() => {
        a.pause();
        a.currentTime = 0;
        a.muted = false;
        a.volume = 1;
      });
  }
}

function play(a: HTMLAudioElement) {
  if (muted) return;
  a.currentTime = 0;
  void a.play().catch(() => {
    /* blocked or no output device — a missing cue must never break the ride */
  });
}

export function playCountdown() {
  play(clips()[0]);
}

export function playEnd() {
  play(clips()[1]);
}

export function isMuted() {
  return muted;
}

export function setMuted(next: boolean) {
  muted = next;
  localStorage.setItem(MUTE_KEY, next ? "1" : "0");
  if (next) for (const a of clips()) a.pause();
}

function ensureVoiceContext(): AudioContext {
  voiceContext ??= new AudioContext();
  return voiceContext;
}

/** Call from a Player gesture when possible; blocked playback remains harmless. */
export function primeVoiceFeedback(): void {
  const context = ensureVoiceContext();
  if (context.state === "suspended") void context.resume().catch(() => undefined);
}

async function playVoiceCue(notes: readonly VoiceCueNote[]): Promise<void> {
  try {
    const context = ensureVoiceContext();
    if (context.state === "suspended") await context.resume();
    const cueStart = context.currentTime + VOICE_CUE_START_DELAY_S;
    let cueEnd = cueStart;
    for (const note of notes) {
      const start = cueStart + note.offsetS;
      const end = start + note.durationS;
      const release = Math.max(start + VOICE_CUE_ATTACK_S, end - VOICE_CUE_RELEASE_S);
      const source = context.createOscillator();
      const gain = context.createGain();
      source.type = "sine";
      source.frequency.setValueAtTime(note.frequencyHz, start);
      gain.gain.setValueAtTime(0, start);
      gain.gain.linearRampToValueAtTime(VOICE_CUE_GAIN, start + VOICE_CUE_ATTACK_S);
      gain.gain.setValueAtTime(VOICE_CUE_GAIN, release);
      gain.gain.linearRampToValueAtTime(0, end);
      source.connect(gain);
      gain.connect(context.destination);
      source.onended = () => {
        activeVoiceSources.delete(source);
        source.disconnect();
        gain.disconnect();
      };
      activeVoiceSources.add(source);
      source.start(start);
      source.stop(end);
      cueEnd = Math.max(cueEnd, end);
    }
    const remainingMs = Math.max(0, (cueEnd - context.currentTime) * 1000);
    await new Promise<void>((resolve) => {
      window.setTimeout(resolve, remainingMs + VOICE_CUE_COMPLETION_GRACE_MS);
    });
  } catch {
    // A blocked or missing audio output must never block voice commands.
  }
}

export function playVoiceReadyCue(): Promise<void> {
  return playVoiceCue(VOICE_READY_CUE);
}

export function playVoiceSuccessCue(): Promise<void> {
  return playVoiceCue(VOICE_SUCCESS_CUE);
}

export function playVoiceErrorCue(): Promise<void> {
  return playVoiceCue(VOICE_ERROR_CUE);
}

export function stopVoiceFeedback(): void {
  for (const source of activeVoiceSources) {
    try {
      source.stop();
    } catch {
      // It may already have reached its scheduled end.
    }
  }
  activeVoiceSources.clear();
}
