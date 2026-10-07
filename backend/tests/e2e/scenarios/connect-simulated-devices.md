---
id: connect-simulated-devices
platform: macos
application: TrainerPro QA.app
---

# Connect simulated devices

## Preconditions

- Use a newly built, packaged TrainerPro QA application, not `tauri dev`.
- Follow the [QA runner guide](../README.md) for identity and profile setup.
- The trainer and heart-rate-monitor slots begin disconnected.
- Do not connect to physical Bluetooth devices.

## Steps

1. Launch TrainerPro QA and wait until its main window is ready.
2. Open the **Devices** tab.
3. Select the shared **Scan for devices** button.
4. Select **Simulated KICKR** from the scan results.
5. Wait until Smart Trainer shows **Simulated KICKR** as connected.
6. Select **Scan for devices** again; connecting the trainer cleared discovery.
7. Select **Simulated HRM** from the scan results.
8. Wait until Heart Rate shows **Simulated HRM** as connected.

## Success

- Smart Trainer shows Simulated KICKR connected.
- Heart Rate shows Simulated HRM connected.
- Both connections are visible simultaneously.
- No connection-error toast is visible.

When every assertion holds, report:

```text
PASS connect-simulated-devices: trainer and HRM simulators connected
```

Otherwise report `FAIL connect-simulated-devices`, the failed step, and the
observed state. A screenshot may be retained for failure diagnosis, but is not
part of the assertion.
