# TrainerPro roadmap

**Purpose:** Record unfinished product outcomes, their order, and external
gates. **Audience:** Maintainers choosing the next line of work.

[PRODUCT.md](PRODUCT.md) owns direction and open product questions;
[SPEC.md](SPEC.md) owns current behavior. This roadmap has no date commitments.
Manual checks below remain open until supported by live evidence.

## Now — desktop reliability and release readiness

- Validate Zwift Ride controls on physical hardware, including pairing, button
  mappings, interrupted holds, recovery, and firmware compatibility.
- Verify the estimated speed/distance Settings option in packaged TrainerPro QA.
- Validate estimated indoor speed/distance with a disposable Garmin upload,
  including speed graphs, lap totals, and pause timing.
- Complete Windows BLE and packaged-app validation.
- Establish macOS signing/notarization and Windows signing before broad releases.

## Voice follow-ups

These are follow-ups, not additional release gates.

- Extend voice commands beyond the Player, with actions appropriate to each
  screen and availability checked against its current state.
- Improve recognition and command reliability under fan noise, varied phrasing,
  and microphone changes, reducing missed commands and unintended actions.
- Complete the [spoken-command](../backend/tests/e2e/scenarios/voice-player-simulated-workout.md)
  and [runtime-discontinuity](../backend/tests/e2e/scenarios/voice-runtime-discontinuities.md)
  scenarios, including speaker echo, focus, sleep/wake, input replacement,
  permission revocation, and continued pointer/keyboard operation.
- Measure long-session resource use, Player responsiveness, trainer control,
  and end-of-speech-to-action latency on the intended hardware floor.
- Validate voice on Windows; macOS results do not establish Windows support.
- Pursue the [Moonshine upstream follow-ups](../vendor/moonshine-wasm/README.md#upstream-follow-ups).
- Build an evaluation corpus only if field accuracy warrants it. The dataset
  and model repositories `simoeswolf/TrainerPro` on Hugging Face are reserved,
  not runtime dependencies. Establish consent, provenance, licensing, and held-out
  evaluation first; consider training only if matching and argument parsing
  prove insufficient.

## Conditional — broader distribution

- Design production Intervals.icu OAuth if distribution expands beyond personal
  use.

## Later — routes from outdoor rides

- Reconstruct terrain from an outdoor ride and advance along it using current
  power and the pure motion model.
- Define smoothing, positioning, braking, trainer resistance, reproducible
  recording, and live speed/distance displays before implementing routes.

## Later — adaptive coaching

- Develop and integrate a separate AI planning service as a plan source and,
  when supported, an Activity destination, following
  [the provider direction](PRODUCT.md#provider-direction).
- Evaluate recommendation quality, privacy, continuity, and operating cost
  before broader distribution.
