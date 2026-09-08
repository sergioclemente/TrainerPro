# Packaged-app end-to-end tests

These tests exercise the bundled TrainerPro QA application as a user would.
They cover the complete application boundary: the frontend, Tauri IPC, the
Rust backend, simulator connections, persistence, and macOS packaging.

The scenarios are structured natural-language instructions for a Computer Use
agent. They are not `cargo test` targets and are not backend-only integration
tests.

## Running a scenario

From the repository root, build the isolated QA application:

```bash
npm run tauri:qa:build -- --bundles app
```

Use the generated `TrainerPro QA.app`. Read `CFBundleExecutable` from its
`Contents/Info.plist` when the executable itself is needed; do not infer its
name from the bundle name.

Give the Computer Use agent one file from [`scenarios`](scenarios) and ask it
to execute the scenario against the packaged application. Environment
preconditions are runner responsibilities rather than test steps.

For a scenario that requires unpaired devices, prepare the QA profile through
the application itself: open **Devices**, use **Forget** for any saved trainer
and HRM, close the application, and then begin the test with a fresh launch.
This setup is not part of the scenario result. Never clear the regular
TrainerPro profile to prepare an E2E run.

## Result contract

A run passes only when the agent completes every step and observes every final
assertion at the same time. Report the scenario ID and either `PASS` or `FAIL`.
A failure report must include the failed step and the visible state that
contradicted the expectation.

Screenshots are not required for successful runs. The agent already observes
the application visually; a runner may retain its final screenshot only when
a run fails and the image is useful for diagnosis.

This suite initially uses only simulated devices. Physical BLE behavior
remains a manual validation gate.
