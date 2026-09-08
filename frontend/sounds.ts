// Interval cues for the player. The two clips come from WorkoutPlanner
// (public/countdown.wav, public/end.wav): a 3.5 s countdown into the next
// interval, and a chime when the workout is over.

/** How far ahead of a segment boundary the countdown clip starts. */
export const COUNTDOWN_LEAD_S = 3.5;

const MUTE_KEY = "trainerpro.sounds.muted";

let muted = localStorage.getItem(MUTE_KEY) === "1";
let countdown: HTMLAudioElement | null = null;
let end: HTMLAudioElement | null = null;
let primed = false;

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
