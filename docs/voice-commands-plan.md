# Voice commands plan

Status: Player-only MVP and workout timeline implemented; release readiness remains
Last updated: 2026-09-22

This is the active task list for local voice commands. Durable architecture
lives in [`architecture.md`](architecture.md), and settled alternatives live in
[`ALTERNATIVES.md`](ALTERNATIVES.md). Moonshine package provenance and rebuild
instructions live in [`vendor/moonshine-wasm/README.md`](../vendor/moonshine-wasm/README.md).

## Product scope

V1 will ship disabled by default, with permission-gated, hands-free control of an active
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
- A workout rail with compact device state, a ride/command timeline, an
  adaptive voice composer, and short nonverbal cues.
- A persistent Settings enable/disable toggle; microphone permission alone
  must not enable voice. This release-policy change is pending implementation.

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
- Browser noise suppression is requested by default to address fan noise.
  Echo cancellation and automatic gain control remain disabled because
  packaged WKWebView testing made Camo effectively silent with echo
  cancellation enabled. There is no automatic raw-capture fallback.
- Moonshine receives `pause,resume,skip,intensity,ERG,listening` as keyterms
  with boost `4.0`. Small Streaming, VAD defaults, and semantic threshold `0.70`
  are unchanged. The user reports improved noise handling and term recognition
  in manual testing, with no observed harm at boost `4.0`; retain that value.
  This is field feedback, not a controlled accuracy benchmark.
- Only completed Moonshine lines enter semantic routing. Partial text appears
  transiently in the voice composer but is not persisted or traced.
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

- The normal 220 px navigation rail shows compact trainer and heart-rate cards.
  Device names are visible; role and connection state remain in accessible
  labels and semantic color rather than repeated text.
- The Player uses a 280 px `RideEventRail`: connections at the top, a
  bottom-anchored ride timeline in the middle, and the voice composer at the
  bottom. Scrolling up pins history and exposes a return-to-latest control.
- User actions from voice, buttons, and keyboard shortcuts appear on the right
  with the same canonical label. Raw voice transcripts remain transient in the
  composer and rejected attempts never enter history. Ride phases, coach text,
  interval results, trainer loss or recovery, and persistent voice failures
  appear on the left. A user-driven start/pause/resume suppresses the matching
  phase acknowledgement.
- Interval results are memory-only for the current ride and use the runtime's
  authoritative ridden time and 1 Hz power/cadence samples. Cards show actual
  averages, prescribed zone shading (including ramp gradients), and explicit
  skipped progress.
- The composer shows actual microphone level while listening, Moonshine's
  partial/final text while processing, and transient rejection plus persistent
  permission/model/retry states. On rejection it holds the recognized text and
  “No matching command” together before returning to the waveform. It has no
  redundant Voice label.
- Connection cards continue to read the existing device status streams; the UI
  does not mirror connectivity state.

## Current evidence

- Fast frontend suite: 36 tests covering the state machine, surface registry,
  shared controls, semantic client timeout/discard behavior, matcher contract,
  number parsing, phase validation, Player command dispatch, and timeline
  ordering/reset/suppression behavior.
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
- A production ride on September 21, 2026 kept one Voice generation healthy
  for 84 minutes against a physical trainer. Start, pause, resume, and skip
  dispatched successfully with no router errors, timeouts, or stale results;
  one input-device change recovered automatically. Post-transcription routing
  averaged 215 ms (198-252 ms). A loud fan coincided with repeated unmatched
  utterances, so noise robustness remains a separate follow-up rather than a
  runtime-lifecycle failure.
- The repeatable human-microphone scenario is
  [`backend/tests/e2e/scenarios/voice-player-simulated-workout.md`](../backend/tests/e2e/scenarios/voice-player-simulated-workout.md).
  Its full run is intentionally deferred to final QA rather than blocking
  implementation.

### Failure tracing

Each app launch writes the normal Rust and frontend tracing stream to
`trainerpro.log` in that app identity's standard log directory. Voice adds only
the pipeline boundaries needed to diagnose a dropped command:
`voice_line_finalized`, `voice_route`, `voice_dispatch`, and `player_phase`,
plus exceptional `voice_restart` / `voice_error` lines. `voice_capture_started`
records requested noise suppression and the track's effective noise suppression,
echo cancellation, and automatic gain control (or `unknown` when unreported).
Events use logfmt fields
and never include audio, transcripts, matched phrases, workout data, ride
measurements, device identity, or paths. CPU and memory investigation remains
external to the application.

## Active release backlog

### ASAP release sequence

- [ ] Rebase the feature branch onto the latest upstream main HEAD, preserving
  local work and resolving any integration conflicts before final verification.
- [ ] Ship Voice disabled by default; update the setting default, focused tests,
  and durable documentation. Verify permission alone does not activate Voice.
- [ ] Correct the stale conformance label expectation (End ride versus Pause
  workout) and rerun focused verification.
- [ ] Complete targeted final QA, review and commit the feature, then obtain
  the user's final personal QA approval before opening the PR.

Performance measurements are explicitly deferred until after release and are
not a gate for this disabled-by-default MVP.

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
- [x] Manually validate representative ride-affecting commands with a physical
  trainer; simulator success is not the hardware gate.
- [ ] Record Windows voice validation as deferred unless Windows joins the
  release target.

## Deferred performance measurements

- [ ] Measure long-session memory, CPU, main-thread delay, player-event
  responsiveness, and trainer control with both models loaded.
- [ ] Measure end-of-speech-to-action latency on the intended hardware floor.

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
