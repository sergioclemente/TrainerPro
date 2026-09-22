# Voice flow

Voice is a local command path with one shared runtime and one active,
screen-owned command surface. Only the Player registers a surface today. Voice
has no wake prefix, does not run in the workout Builder, and never gives a
model direct access to application behavior.

```text
voice_enabled + voice surface active + app focused + microphone permission
                              |
                              v
                    WorkerMicTranscriber
 system-default mic -> AudioWorklet -> Moonshine STT worker
                              |
              partial transcript -> voice composer only
                    completed transcript
                              |
                              v
                    semanticRouter.worker
 transcript embedding -> active surface phrases -> semantic match only
                              |
                    zero or one intent match
                              |
                              v
       screen-owned command registry -> existing application action
```

`VoiceController.tsx` is the application-level production owner. It is the
production caller of `reduceVoiceMachine`, automatically requests permission,
loads and retains both local models, starts and suspends capture based on the
active voice surface and application activity, admits one completed utterance,
and serializes routing, execution, composer feedback, and stream reset. Partial
and finalized transcripts remain in React memory only and are cleared when the
controller returns to listening. `voiceMachine.ts`
remains the pure lifecycle reducer and invalidates asynchronous work when the
setting, permission, active surface identity, or application focus changes.
Where the WebView exposes the microphone Permissions API, the controller also
observes permission changes. Focus, visibility, page lifecycle, and permission
boundaries synchronously mute capture and invalidate pending transcripts and
semantic results before the React state transition completes. The capture
adapter treats an ended or muted input track, a media-device list change, or a
non-running AudioContext as a discontinuity. It admits only the first signal
from a capture generation, tears down that stream, and reacquires the current
system-default input. A reset that was already in flight cannot unmute an
invalidated capture. One clean automatic restart is allowed while the current
Player context remains active; another discontinuity becomes a persistent
error until Retry. Revocation, hard disable, and persistent runtime failure
release both local runtimes and clear transient feedback; leaving the Player
only stops capture so the models remain warm.

`workerMicTranscriber.ts` owns browser microphone and WebAudio lifecycle. Its
AudioWorklet transfers mono PCM to Moonshine's STT worker; a
`ScriptProcessorNode` is the compatibility fallback. Moonshine owns resampling,
VAD, streaming state, and completed transcript lines.

Moonshine uses the workout keyterm list defined in `workerMicTranscriber.ts`
with a boost of `4.0` to favor workout vocabulary during decoding. Capture
requests browser noise suppression; echo cancellation and automatic gain control
remain off for WKWebView/Camo compatibility. After capture starts,
`voice_capture_started` logs requested noise suppression and the track's effective
noise suppression, echo cancellation, and gain-control settings. Unreported
settings are `unknown`; a successful request alone does not prove suppression
is active. No audio, transcript, or device identity is logged.

`semanticRouter.worker.ts` loads the bundled EmbeddingGemma model, caches phrase
embeddings by text, and selects the closest command key from the catalog
supplied by the active surface. It returns only the semantic match and cannot
parse or execute application actions. `semanticRouterClient.ts` owns worker
request serialization and admits no queue. Routing is bounded to two seconds,
about nine times the current local-model p95; a timeout terminates the
worker so its late result cannot execute. The controller discards that
utterance and rebuilds the semantic worker once. A second consecutive failure
becomes a persistent Voice error until Retry. `voiceSurface.ts` defines the
boundary between this shared runtime and screen-owned command policy.

`commandRegistry.ts` derives each surface's semantic catalog, availability,
dispatch, and live-context lookup from one declarative command registry. A
command key is declared only once; paraphrases are examples belonging to that
command, not separate actions. Routing always compares against the full active
screen catalog before availability is checked, so an unavailable command cannot
be reinterpreted as a different available command. `spokenNumber.ts` compiles
command-specific finite integer domains, so embeddings choose meaning while
deterministic code extracts and validates numeric slots.

`voiceControls.ts` declares shared, session-scoped command suspension. “Stop
listening” and “stop talking” pause application voice commands without stopping
the microphone, Moonshine, or semantic model; while paused, only “resume
listening” and its aliases can execute. `voiceRouting.ts` composes these controls
with the active screen catalog and enforces the gate for both production and
conformance testing. The persistent Settings toggle remains the hard disable.

`screens/Player.voice.ts` declares the Player commands, phrases, numeric domains,
known transcription recovery, phase availability, live-state validation, and a
narrowed set of injected IPC capabilities. It has no `endRide`; end-like
language can only prepare the `pause` command and visible “finish manually”
guidance in the transient composer.
`Player.tsx` registers that stable surface while an unfinished workout is
active. Future screens add adjacent `.voice.ts` modules rather than extending
the semantic worker with application behavior. `rideTimeline.ts` is the pure,
memory-only ordering owner for canonical user actions and ride events.
`screens/Player.actions.ts` is used by voice, buttons, and keyboard shortcuts,
so equivalent actions create the same entry and phase correlation without
coupling command policy to the view. Recognition text and rejected attempts
remain transient in the composer rather than entering the timeline.
`RideEventRail.tsx` renders that chat-like timeline between compact connection
cards and the adaptive `VoiceComposer`. The backend emits one `segment_result`
at each segment boundary from the same ridden-time ticks and 1 Hz measurements used
by the ride recorder. Voice remains Player-only; normal screens show connection
status without an inactive Voice composer.

`voice-models.manifest.json` pins both local model sets. `modelAssets.ts`
resolves their packaged Tauri resource URLs. A missing or invalid speech or
command model is reported as a source-specific persistent Voice error; the
controller disposes partial runtimes without failing the surrounding app.
Worker request and response types live in `semanticRouterProtocol.ts`.

## Packaging, startup, and offline behavior

Voice is self-contained. The application bundle contains Moonshine, the
English Small Streaming speech model, EmbeddingGemma, both tokenizers, and all
voice runtime code. `modelAssets.ts` resolves these files through Tauri's local
resource protocol; the voice path has no model download, update, hosted
inference, or other external network request. After TrainerPro is installed,
voice commands work without an internet connection. This claim covers the
voice path, not opt-in remote workout sources.

Voice is disabled by default; enable it explicitly in Settings. A saved choice
is preserved, and microphone permission alone does not enable voice.
The first time an enabled voice surface becomes active and the app has focus,
TrainerPro requests microphone permission if needed, loads both bundled models
in parallel, prepares the active command catalog, starts capture, and plays the
ready cue. The voice composer shows preparation and the model source currently
loading. The latest Node reference measurement on the development machine was
411 ms to load EmbeddingGemma and 7.315 s to prepare the first full command
catalog; this is diagnostic evidence, not a startup-time guarantee for every
WebView or Mac. Leaving the Player stops capture but retains the loaded models,
so returning to the Player avoids a cold model load. Disabling voice,
permission revocation, a persistent runtime failure, or application shutdown
releases the runtimes; the next activation is cold again.

Transient microphone or semantic-worker failures each receive one clean
automatic restart. A repeated failure becomes a persistent Voice error and
stops command execution. `Retry` in the Player's voice composer disposes stale
state and starts a fresh permission/model/capture cycle. The persistent
`Enable voice commands` setting under Settings > Basic info is the hard-disable
path; disabling it releases both runtimes and microphone capture while the
rest of TrainerPro remains usable.

Packaging measurements from the unsigned, optimized QA build on September 17,
2026:

- Bundled model payload: 342,562,690 bytes (326.7 MiB).
- Installed `.app`: 353 MiB on disk.
- Compressed arm64 DMG: 297,460,645 bytes (283.7 MiB).
- DMG SHA-256: `38b9d89d8f7efe828f40d479278230e7ac6d2194f20ad47ed080e6b3859d16f1`.
- Architecture: Apple Silicon (`arm64`) only for this build.
- Deployment floor: macOS 11.0, declared by `LSMinimumSystemVersion` and the
  executable's Mach-O load command. This means M1 or newer Apple Silicon
  hardware; it is a compatibility floor, not yet a measured latency or memory
  performance floor.

The mounted installer was checked for its QA bundle identifier, microphone
purpose string, model directories, notices, and byte-identical bundled legal
documents. Signing and notarization can change installer size and checksum, so
release artifacts must publish their own measurements.

`npm run voice:conformance` loads the pinned local EmbeddingGemma artifact and
Moonshine WASM in Node, derives canonical cases from the live Player and shared
control registries, and runs the versioned curated corpus in
`Player.voice.conformance.json` through the same matcher and command preparation
used in production. It emphasizes positive paraphrases, numeric behavior,
phase validity, and command-confusion pairs. It is intentionally separate from
the fast default frontend tests because it loads the full model.

Ready, success, and rejection/error feedback use short synthesized nonverbal
cues; they are independent of the workout-sound mute and are best-effort under
WebView autoplay policy. The human-microphone
`backend/tests/e2e/scenarios/voice-player-simulated-workout.md` scenario covers
every Player action through the simulator-backed production path. Raw audio and
transcripts are not persisted.
