# Voice developer guide

**Purpose:** Orient developers working on the voice pipeline.
**Audience:** Contributors navigating capture, routing, and command execution.

## Flow

```text
PTT hold -> microphone -> Moonshine worker -> release-finalized transcript
                                      |
partial text -> composer              v
                             semantic worker -> intent
                                                |
                                  screen command registry
                                                |
                                  shared Player action -> IPC
```

Start with [VoiceController.tsx](../frontend/voice/VoiceController.tsx), the production caller of
the lifecycle reducer. Follow capture through
[workerMicTranscriber.ts](../frontend/voice/workerMicTranscriber.ts), matching through
[semanticRouter.worker.ts](../frontend/voice/semanticRouter.worker.ts), and command policy through
[Player.voice.ts](../frontend/screens/Player.voice.ts). Workers recognize speech or match
intents; application-owned command preparation validates arguments and live
state before execution.

[Player.actions.ts](../frontend/screens/Player.actions.ts) is the shared entry point for
voice, buttons, and keyboard actions.
[rideTimeline.ts](../frontend/rideTimeline.ts) orders their session-only feedback.
Command phrases, numeric domains, model options, and timing constants belong in
their owning source files, not this guide.

## Canonical references

- [Behavior specification](SPEC.md#workout-voice-and-timeline):
  activation, privacy, supported actions, and user feedback.
- [Architecture](architecture.md#local-workout-voice):
  lifecycle ownership and routing invariants.
- [Model manifest](../frontend/voice/voice-models.manifest.json): pinned runtime and model assets.
- [Moonshine provenance](../vendor/moonshine-wasm/README.md):
  fork rationale, rebuild instructions, and upstream follow-ups.
- [Spoken-command QA](../backend/tests/e2e/scenarios/voice-player-simulated-workout.md)
  and [runtime-discontinuity QA](../backend/tests/e2e/scenarios/voice-runtime-discontinuities.md):
  repeatable manual validation.
- [Roadmap](ROADMAP.md#voice-follow-ups): unfinished work.

Run `npm run test:frontend` for focused tests. Run
`npm run voice:conformance` for model-backed routing checks against the
registry-derived cases and curated corpus; it checks text routing, not microphone
or transcription accuracy.
