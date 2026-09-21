# Voice commands plan

Status: Player-only MVP implemented; release readiness remains
Last updated: 2026-09-20

This is the active task list for local voice commands. Durable architecture
lives in [`architecture.md`](architecture.md), and settled alternatives live in
[`ALTERNATIVES.md`](ALTERNATIVES.md). Moonshine package provenance and rebuild
instructions live in [`vendor/moonshine-wasm/README.md`](../vendor/moonshine-wasm/README.md).

## Product scope

V1 provides default-on, permission-gated, hands-free control of an active
workout. Audio, transcription, intent matching, and argument parsing remain on
the device. Raw audio and transcripts are not persisted or sent anywhere.

V1 includes:

- System-default microphone capture while the Player is visible and the app is
  active.
- Local Moonshine Voice v2 Small Streaming transcription.
- Local EmbeddingGemma semantic matching with deterministic bounded argument
  parsing.
- Start, pause, resume, skip, absolute/relative intensity, and explicit ERG
  control through the existing typed Player IPC.
- Session-scoped “stop listening” / “resume listening” voice controls.
- Visible status, transient command results, and short nonverbal cues.
- A persistent Settings opt-out; microphone permission otherwise enables voice
  automatically.

Deferred until real Player usage justifies expansion:

- Voice on the workout Builder or any non-Player screen.
- A microphone picker, background listening, wake word, or spoken prefix.
- Cloud processing, general conversation, coaching, multi-step actions, or a
  generative command model.
- Runtime TTS. Synthetic TTS is reserved for future offline evaluation only.
- Training or shipping a TrainerPro-specific model.

## Implemented design

### Runtime and ownership

- `VoiceController` is the single lifecycle owner for permission, capture,
  model readiness, routing, feedback, suspension, and cleanup.
- `WorkerMicTranscriber` owns thin WebAudio capture and transfers mono PCM to
  Moonshine's STT worker. Moonshine owns resampling, VAD, streaming state, and
  completed transcript lines.
- The semantic worker owns EmbeddingGemma, cached phrase embeddings, and
  nearest-intent selection. It cannot parse arguments, invoke IPC, or mutate
  application state.
- `Player.voice.ts` owns the Player command registry, representative phrases,
  typed argument preparation, phase availability, live-state validation, and
  narrowed IPC capabilities.
- The complete Player and shared-control catalogs are matched first. The
  registries then reject unavailable commands, preventing an unavailable
  phrase from being reinterpreted as another action.
- Every asynchronous operation is tied to a controller generation. Screen,
  focus, setting, permission, input, and failure changes invalidate stale work.
- There is at most one routing request and one application action per completed
  utterance. Busy-period audio is discarded rather than queued.

### Capture and lifecycle

- Capture uses the system-default mono input through `getUserMedia`.
- Echo cancellation, noise suppression, and automatic gain control are
  disabled because packaged WKWebView testing made the Camo input effectively
  silent with echo cancellation enabled.
- Only completed Moonshine lines enter semantic routing. Partial text is not
  displayed or persisted.
- Utterances are bounded to 15 seconds. Reaching the bound rejects the line and
  resets the Moonshine stream.
- Leaving the Player or losing application focus stops capture and discards
  partial audio while retaining warm models.
- Hard disable, permission revocation, ended capture, persistent runtime error,
  and application shutdown release both model runtimes and transient state.
- Supported WebViews are observed through the microphone Permissions API;
  ended tracks provide the fallback signal.

### Routing and recovery

- The intent schema is versioned in `voice-models.manifest.json`; the current
  version is 4.
- Routing has a two-second timeout, about nine times the roughly 220 ms local
  model p95. Timeout terminates the semantic worker, so a late result cannot
  execute.
- The interrupted utterance is discarded. The controller rebuilds the semantic
  worker once while retaining the speech runtime.
- A consecutive failure becomes a persistent Voice error until the rider uses
  Retry. There is no restart loop.
- Numeric slots accept finite declared domains. Player intensity supports
  50–150% absolute values and 1–10 percentage-point relative adjustments.
- The existing Player behavior remains authoritative for normalized intensity
  and trainer actions.

### Command and safety contract

Player commands are available only in applicable live phases:

| Command | Ready | Riding | Paused | Behavior |
|---|:---:|:---:|:---:|---|
| Start | yes | no | no | Existing `startRide` IPC |
| Pause / stop / take a break | no | yes | no | Existing `pauseRide` IPC |
| Resume / continue | no | no | yes | Existing `resumeRide` IPC |
| Skip interval | no | yes | yes | Existing `skipSegment` IPC |
| Set or adjust intensity | yes | yes | yes | Existing absolute `setIntensity` IPC |
| ERG on/off | yes | yes | yes | Existing `setErg` IPC |

Safety invariants:

- No voice capability can invoke `endRide`. End/finish/stop-ride language maps
  to pause and tells the rider to finish manually.
- Unknown, incomplete, invalid, unavailable, or unmatched input has no
  application side effect.
- Prepared commands re-read live Player state immediately before execution.
- “Stop” remains a Player pause. “Stop listening” suspends application voice
  commands while capture stays active for “resume listening.”
- An application-action failure uses the existing error toast and voice error
  feedback; voice never creates a second backend command channel.

### Status UI

- Voice status appears only in the Player's 200 px `RideEventRail`.
- The normal 200 px navigation rail shows trainer and heart-rate connections,
  but not an inactive Voice status.
- Voice, trainer, and HRM use one presentation-only status-card pattern with
  code-native icons, explicit state text, accessible labels, and semantic tone.
- The Player rail keeps transient command feedback above the bottom status
  cluster. Feedback remains ephemeral and does not become a toast history.
- Connection cards continue to read the existing device status streams; the UI
  does not mirror connectivity state.

## Current evidence

- Fast frontend suite: 32 tests covering the state machine, surface registry,
  shared controls, semantic client timeout/discard behavior, matcher contract,
  number parsing, phase validation, and Player command dispatch.
- Model-backed conformance: 71/71 cases (35 registry-derived canonical and 36
  curated) at the provisional 0.70 threshold.
- Latest Node measurement: 411 ms model load, 7.315 s catalog warmup, 206 ms
  median route, 223 ms p95 route, and approximately 1.376 GiB RSS growth.
- Debug QA microphone runs exercised start, pause, resume, skip, absolute and
  relative intensity, plus end-like-to-pause behavior against Simulated KICKR.
- The production model set is 342,562,690 bytes before application overhead.
  The measured unsigned arm64 QA application was 353 MiB on disk and its
  compressed DMG was 297,460,645 bytes (283.7 MiB).
- Packaged QA failure injection on September 18, 2026 independently removed
  EmbeddingGemma's `model_q4.ort` and Moonshine's `encoder.ort`. Each produced
  the expected source-specific persistent Voice error; Retry remained bounded
  to Voice, and pointer intensity control continued to update the Player.
- A fresh release-mode QA bundle on September 18, 2026 used the exact
  `com.trainerpro.desktop.qa` identity, connected Simulated KICKR, loaded both
  application-local models to **Listening**, and retained pointer intensity
  plus keyboard start control with Voice disabled. Voice was re-enabled after
  the smoke test. The matching production bundle was also rebuilt without the
  simulator feature.
- The repeatable human-microphone scenario is
  [`backend/tests/e2e/scenarios/voice-player-simulated-workout.md`](../backend/tests/e2e/scenarios/voice-player-simulated-workout.md).
  Its full run is intentionally deferred to final QA rather than blocking
  implementation.

### Failure tracing

Each app launch writes the normal Rust and frontend tracing stream to
`trainerpro.log` in that app identity's standard log directory. Voice adds only
the pipeline boundaries needed to diagnose a dropped command:
`voice_line_finalized`, `voice_route`, `voice_dispatch`, and `player_phase`,
plus exceptional `voice_restart` / `voice_error` lines. Events use logfmt fields
and never include audio, transcripts, matched phrases, workout data, ride
measurements, device identity, or paths. CPU and memory investigation remains
external to the application.

## Active release backlog

### Product and legal

- [x] Add the Settings subtitle: “Processed on this device. Audio and
  transcripts are not stored or sent.”
- [x] Verify the macOS microphone-purpose text remains: “TrainerPro uses the
  microphone for hands-free workout commands. Voice processing happens on this
  device.”
- [x] Add an in-app About surface and bundled `NOTICE.txt` inventory for
  Moonshine, EmbeddingGemma, ONNX Runtime, tokenizers, and compiled runtime
  dependencies.
- [x] Pin and bundle the applicable Gemma agreement and Prohibited Use Policy,
  and include their use restrictions as enforceable provisions of TrainerPro's
  model-specific binary distribution terms.
- [x] Document bundled size, first-load behavior, offline behavior, and the
  persistent Retry/Disable recovery path.
- [x] Measure the compressed installer and record the supported Apple Silicon
  macOS hardware/OS floor from packaged results.

### Runtime hardening

- [x] On an unexpected ended/default-input track, make one clean automatic
  reacquisition attempt while the Player is active; a repeated failure remains
  a persistent error.
- [x] Enforce fresh capture/stream generations across focus, page lifecycle,
  sleep/audio interruption, device-list, track, and permission boundaries so
  stale audio and semantic results cannot execute.
- [ ] Run the human-microphone `voice-runtime-discontinuities` QA scenario for
  focus, sleep/wake, default-input replacement, and permission revocation.
- [x] Cover missing or invalid speech and embedding model-load failures with a
  source-specific persistent Voice error while the rest of TrainerPro remains
  usable.
- [x] Validate missing speech and embedding resources by tampering with the
  packaged QA application, and verify the surrounding Player remains usable.
- [ ] Measure long-session memory, CPU, main-thread delay, player-event
  responsiveness, and trainer control with both models loaded.
- [ ] Measure end-of-speech-to-action latency on the intended hardware floor.

### Final validation

- [ ] Run the complete simulator-backed spoken-command QA scenario.
- [ ] Verify countdown, end, and voice cues through speakers for echo-driven
  false activation and missed commands.
- [x] Verify pointer and keyboard controls while Voice is disabled, and pointer
  controls while Voice is in a persistent model-load error.
- [ ] Verify pointer and keyboard controls while microphone permission is
  denied or revoked.
- [x] Build the release TrainerPro QA `.app`, verify its exact QA bundle
  identity, and validate packaged permission plus application-local model
  loading.
- [ ] Manually validate ride-affecting commands with a physical trainer;
  simulator success is not the hardware gate.
- [ ] Record Windows voice validation as deferred unless Windows joins the
  release target.

## Deferred evaluation program

The Hugging Face repositories `simoeswolf/TrainerPro` (dataset and model repo
types) remain reserved for future evaluation and possible training. Neither is
an application runtime dependency.

Before generating or uploading a dataset:

- Write the dataset card, schema, provenance, consent, and license policy.
- Pin one evaluation-only TTS revision and record its license; do not bundle it
  with TrainerPro.
- Generate canonical commands, positive paraphrases, numeric minimal pairs,
  neighboring-command confusions, context-invalid cases, and a small ordinary-
  speech sanity set.
- Preserve source text, audio provenance, raw Moonshine transcript, normalized
  transcript, expected intent/arguments/action, and categorical failure tags.
- Split by command-template family and voice; keep consented real recordings in
  a separate held-out set.
- Tune the semantic threshold only on a development split, then report intent,
  argument, rejection, context, and safety-critical false-execution accuracy
  separately.
- Train or publish a command model only if this evidence shows the semantic
  matcher plus deterministic parsers cannot meet the product requirement.

## Deferred Moonshine upstreaming

Do not block TrainerPro voice release on upstreaming. The canonical source is
commit `663c475a86e5d738041646c7a522160db6870839` on
`simoeswolf/moonshine:trainerpro-v0.1.5-wasm-compat`.

- [ ] Upstream the minifier-safe generated AudioWorklet source fix with a
  production-bundle regression test.
- [ ] Upstream `processorerror` propagation so a render failure cannot appear
  as successful zero-audio capture.
- [ ] Upstream best-effort Cache Storage handling for non-HTTP application
  model URLs.
- [ ] Upstream explicit worker and exact WASM asset URL support for bundlers.
- [ ] Upstream worker RPC error-stack propagation.
- [ ] Ask Moonshine to publish a maintained single-thread WASM package for
  WebViews without `SharedArrayBuffer`.
- [ ] Adopt the first suitable upstream release, validate it in packaged
  TrainerPro QA, and then remove the vendored tarball.

## Change log

- 2026-09-20: Rebased onto the merged canonical workout/data model and added a
  small per-launch structured trace for locating dropped voice commands.
- 2026-09-18: Rebuilt and audited release-mode QA and production application
  bundles. The packaged QA smoke test reached **Listening** with local models,
  connected the simulator, and kept pointer and keyboard controls functional
  with Voice disabled.
- 2026-09-17: Reconciled this document with the implemented Player-only MVP,
  removed abandoned command-model spike history, corrected full-catalog-first
  routing, and reorganized remaining work into release, evaluation, and
  upstream milestones. Replaced the small emoji status rows with shared,
  accessible status cards; Voice remains Player-only.
- 2026-09-17: Added permission-change and ended-track handling, two-second
  semantic timeout, one semantic-worker restart, persistent repeated-failure
  UX, and explicit runtime cleanup.
- 2026-09-16: Added declarative screen-owned command registries, deterministic
  numeric slots, shared hands-free suspension, model-backed conformance, and
  the simulator-backed QA scenario.
- 2026-09-14: Validated the local microphone-to-command path in debug QA,
  retained unprocessed capture for WKWebView/Camo compatibility, and verified
  end-like language pauses rather than finishes a simulated ride.
- 2026-09-11: Published the canonical Moonshine WebView compatibility commit
  and retained only the derived single-thread package plus provenance here.
- 2026-09-08: Began the local voice-command design.
