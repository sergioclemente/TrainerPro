# Voice developer guide

**Purpose:** Orient developers working on the voice pipeline.
**Audience:** Contributors navigating capture, routing, and command execution.

## Flow

```text
microphone -> Moonshine worker -> completed transcript
                                      |
partial text -> composer              v
                             semantic worker -> intent
                                                |
                                  screen command registry
                                                |
                                  shared Player action -> IPC
```

Start with [VoiceController.tsx](VoiceController.tsx), the production caller of
the lifecycle reducer. Follow capture through
[workerMicTranscriber.ts](workerMicTranscriber.ts), matching through
[semanticRouter.worker.ts](semanticRouter.worker.ts), and command policy through
[Player.voice.ts](../screens/Player.voice.ts). Workers recognize speech or match
intents; application-owned command preparation validates arguments and live
state before execution.

[Player.actions.ts](../screens/Player.actions.ts) is the shared entry point for
voice, buttons, and keyboard actions.
[rideTimeline.ts](../rideTimeline.ts) orders their session-only feedback.
Command phrases, numeric domains, model options, and timing constants belong in
their owning source files, not this guide.

## Canonical references

- [Behavior specification](../../docs/SPEC.md#workout-voice-and-timeline):
  activation, privacy, supported actions, and user feedback.
- [Architecture](../../docs/architecture.md#local-workout-voice):
  lifecycle ownership and routing invariants.
- [Model manifest](voice-models.manifest.json): pinned runtime and model assets.
- [Moonshine provenance](../../vendor/moonshine-wasm/README.md):
  fork rationale, rebuild instructions, and upstream follow-ups.
- [Spoken-command QA](../../backend/tests/e2e/scenarios/voice-player-simulated-workout.md)
  and [runtime-discontinuity QA](../../backend/tests/e2e/scenarios/voice-runtime-discontinuities.md):
  repeatable manual validation.
- [Roadmap](../../docs/ROADMAP.md#voice-follow-ups): unfinished work.

Run `npm run test:frontend` for focused tests. Run
`npm run voice:conformance` for model-backed routing checks against the
registry-derived cases and curated corpus; it checks text routing, not microphone
or transcription accuracy.
