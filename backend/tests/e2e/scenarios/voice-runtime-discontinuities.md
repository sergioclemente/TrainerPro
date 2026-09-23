---
id: voice-runtime-discontinuities
platform: macos
application: TrainerPro QA
input: human-microphone
---

# Keep voice commands separate across capture discontinuities

## Purpose

Verify that audio and pending semantic results from before a focus, sleep,
input-device, or permission boundary cannot be joined to audio after it. Each
boundary must start a fresh Moonshine stream before another command can run.

## Preconditions

- Run the current TrainerPro QA debug application. Do not launch or interact
  with the regular TrainerPro application.
- Open an unfinished workout in the Player and wait for the Voice card to show
  **Listening**.
- Voice commands are enabled, microphone permission is granted, and the current
  intensity is **100%**.
- Two working microphone inputs are available for the default-input step.
- Keep the workout not started or paused so the intensity remains easy to
  inspect.

## Rules for split phrases

- For each boundary, say **“increase intensity”** immediately before the
  boundary and **“by five percent”** only after returning to TrainerPro and
  observing **Listening** again.
- Neither half is a valid command by itself. If the intensity changes to 105%,
  audio crossed the boundary and the step fails.
- After the unchanged-intensity assertion, say the complete phrase
  **“increase intensity by five percent.”** Expect exactly one change to 105%,
  then say **“set intensity to one hundred percent”** and wait for 100% before
  continuing. This proves the post-boundary capture is functional.

## Steps

1. Test application focus. Say the first half, switch focus to another
   application, wait two seconds, return to TrainerPro, wait for **Listening**,
   and say the second half. Expect the intensity to remain 100%, then run the
   complete-phrase check above.
2. Test sleep/wake. Say the first half, put the Mac to sleep, wake and unlock
   it, return to TrainerPro, wait for **Listening**, and say the second half.
   Expect the intensity to remain 100%, then run the complete-phrase check.
3. Test default-input replacement. Say the first half, change the macOS
   system-default input to the other working microphone, return to TrainerPro,
   wait for **Listening**, and say the second half into the new default input.
   Expect the intensity to remain 100%, then run the complete-phrase check
   using the new microphone.
4. Test permission revocation. Say the first half, revoke TrainerPro QA's
   microphone permission in macOS System Settings, and return to TrainerPro.
   Expect **Unavailable** or a persistent Voice error and no intensity change.
   Restore permission, use **Retry**, wait for **Listening**, and say the second
   half. Expect the intensity to remain 100%, then run the complete-phrase
   check.

## Success

- No split phrase changes workout state or produces successful command
  feedback.
- Every boundary returns through **Preparing…** or a clean recovery state
  before **Listening**.
- One complete post-boundary command works exactly once after each recovery.
- Permission revocation leaves pointer and keyboard controls usable.

When every assertion holds, report:

```text
PASS voice-runtime-discontinuities: no audio or command crossed a capture boundary
```

Otherwise report `FAIL voice-runtime-discontinuities`, the boundary, the first
unexpected feedback or state change, and whether the post-boundary complete
command worked.
