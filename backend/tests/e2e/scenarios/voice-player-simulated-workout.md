---
id: voice-player-simulated-workout
platform: macos
application: TrainerPro QA
input: human-microphone
---

# Control a simulated workout by voice

## Purpose

Exercise every Player voice-command action across the production boundary:
microphone capture, Moonshine transcription, semantic routing, typed command
preparation, Tauri IPC, the Rust player runtime, and the simulated trainer.

Parser boundaries, paraphrase breadth, phase rejection, and already-satisfied
no-ops belong to the automated frontend and model-conformance suites. This
scenario checks one representative spoken path per application action.

## Preconditions

- Run the current TrainerPro QA debug application. Use a packaged release QA
  build only when the run is also serving as the release packaging check.
- Do not launch or interact with the regular TrainerPro application.
- **Simulated KICKR** is connected. **Simulated HRM** may also be connected.
  The `connect-simulated-devices` scenario can establish both connections.
- `samples/sweetspot_3x10.zwo` is imported into the QA library as
  **Sweet Spot 3x10**.
- Voice commands are enabled in Settings, macOS microphone permission is
  granted to TrainerPro QA, and the intended system-default microphone is on.
- The room is quiet enough to hear the nonverbal cues and observe whether the
  application accidentally responds to its own audio.

## Rules for spoken steps

- A person speaks each phrase in quotation marks naturally into the microphone.
- Do not click a Player control or use a keyboard shortcut in place of a spoken
  step.
- After each phrase, wait for the command feedback and the corresponding Player
  state change before continuing. A transient success card alone is not enough
  when the step also names a persistent UI assertion.
- If Moonshine produces no completed utterance or the semantic router rejects
  the phrase, report that step as failed; do not repeat it until it happens to
  pass.

## Steps

1. Open **Sweet Spot 3x10**, select **Ride this workout**, and wait until the
   Player rail shows **Listening** and the trainer shows **Simulated KICKR** as
   connected. The Player must show **Start** and **ready to start**.
2. Say **“Set intensity to ninety percent.”** Expect feedback
   **Intensity set to 90%** and a persistent **90% intensity** badge.
3. Say **“Disable ERG mode.”** Expect feedback **ERG disabled** and the Player
   ERG control and target label to show **ERG off**.
4. Say **“Enable ERG mode.”** Expect feedback **ERG enabled**, the ERG control
   to show enabled, and the target label to return to **ready to start**.
5. Say **“Start the workout.”** Expect feedback **Ride started**, the main
   control to change to **Pause**, the elapsed clock to advance, and simulated
   power to appear.
6. Say **“Increase intensity by five percent.”** Expect feedback
   **Intensity set to 95%** and the badge to show **95% intensity**.
7. Say **“Decrease intensity by five percent.”** Expect feedback
   **Intensity set to 90%** and the badge to return to **90% intensity**.
8. Note the active interval and its remaining time. Say **“Next interval.”**
   Expect feedback **Interval skipped** and the graph/interval clock to advance
   to the following interval rather than merely advancing by one normal tick.
9. Say **“Pause.”** Expect feedback **Workout paused**, the main control to
   change to **Resume**, and both elapsed and interval clocks to stop advancing.
10. Say **“Resume.”** Expect feedback **Workout resumed**, the main control to
    change to **Pause**, and the clocks to advance again.
11. Say **“End the ride.”** Expect feedback
    **Workout paused — finish manually**, the main control to change to
    **Resume**, and the Player to remain open. The Summary screen must not open
    and no ride-completion confirmation may appear.
12. End the paused ride manually with the **End ride** button and its
    confirmation. This cleanup action is intentionally not a voice command.

## Success

- Each spoken step produces exactly one expected action.
- Start, pause, resume, skip, absolute intensity, relative intensity in both
  directions, ERG off, and ERG on reach persistent Player/runtime state.
- The connected simulator produces measurements after the spoken start.
- **End the ride** pauses and requires manual completion; it never ends the
  ride directly.
- No connection-error toast or unexpected command feedback appears.

When every assertion holds, report:

```text
PASS voice-player-simulated-workout: all Player voice actions reached the simulated runtime
```

Otherwise report `FAIL voice-player-simulated-workout`, the first failed step,
the exact phrase spoken, and the visible state that contradicted the
expectation. Retain a screenshot only when it helps diagnose the failure.

## Fan-noise follow-up

Use the same QA identity and simulator with Camo as the system-default input.
First verify non-silent capture and one pause/resume cycle in quiet conditions.
Then turn on the workout fan and perform five pause/resume cycles, one skip,
and one intensity command. Count first attempts only; restore the appropriate
phase using the UI after a missed command so the next attempt is valid.
Require at least four of five first-attempt successes for each of pause and
resume, and successful skip/intensity actions. Leave only the fan running for
two minutes while riding and confirm that no command executes.

Inspect `voice_capture_started` in the QA log for effective noise suppression.
`unknown` means the WebView did not report the setting, not that suppression is
active. Record that distinction with the spoken results. Camo silence or a
capture-continuity regression fails this check; revert the noise-suppression
constraint if it causes either. If accuracy misses the target, investigate
with separately consented audio before further tuning.
