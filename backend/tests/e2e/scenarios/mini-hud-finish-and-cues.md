---
id: mini-hud-finish-and-cues
platform: macos
application: TrainerPro QA.app
---

# Finish, cues and targets in the mini HUD

## Purpose

Exercise what only the running application can show: the HUD window growing
for a caption, the target reveals, the end chime, and the hand-over from the
HUD to the summary. The view rules themselves belong to
`frontend/miniHud.test.mjs`.

## Preconditions

- Use a newly built, packaged TrainerPro QA application, not `tauri dev`.
- Follow the [QA runner guide](../README.md) for identity and profile setup.
- **Simulated KICKR** is connected. The
  [`connect-simulated-devices`](connect-simulated-devices.md) scenario can
  establish the connection.
- `samples/sweetspot_3x10.zwo` is imported into the QA library as
  **Sweet Spot 3x10**.
- Sound is on in the Player and the system output is audible.

## Steps

1. Open **Sweet Spot 3x10** and select **Ride this workout**. The stats row
   under the clocks shows five figures on one line and there is no **Stats**
   button in the controls.
2. Click the stats row, then press `d`. Expect the large strip to open in
   place and then collapse back to the single line.
3. Select **Start**, then **Mini**. Expect the main window to hide and the HUD
   to show watts, rpm, bpm and the interval clock.
4. Wait until 30 seconds into the warm-up. Expect a caption below the strip
   reading **Welcome! Ease into it.**, the four cells and their labels still
   visible, and the caption to disappear with the window back to the strip.
   Watch the watts cell meanwhile: the ramp's rising target must not replace
   the reading.
5. Click the watts cell. Expect the target with an arrow and the label
   **target** for about 3 seconds, then the reading again.
6. Hover the HUD and select skip. Expect the watts cell to show the interval's
   target labelled **new target** and the rpm cell to show **90** labelled
   **new target**, both for about 5 seconds.
7. Click the rpm cell. Expect **90** labelled **target** for about 3 seconds.
8. Raise the intensity by 1 % from the bar. Expect the watts cell to show the
   new target for about 5 seconds and the rpm cell to keep its reading.
9. Select skip until the cool-down is the current interval, then skip once
   more to complete the workout. Expect the end chime once, the HUD green with
   ride totals and **Workout complete · summary in N s** counting down, and
   after about 4 seconds the HUD gone and the main window focused on the
   summary.
10. Ride the workout again without **Mini**: start, skip through every
    interval. Expect the end chime once and the summary.

## Success

- Every expectation above was observed.
- The HUD never showed live cells again after the workout completed.
- No error toast is visible.

When every assertion holds, report:

```text
PASS mini-hud-finish-and-cues: cues, targets, chime and summary hand-over
```

Otherwise report `FAIL mini-hud-finish-and-cues`, the failed step, and the
observed state.
