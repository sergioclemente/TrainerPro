# Synthetic FIT decoder fixtures

Purpose: independent binary inputs for engineers testing the shared FIT decoder.

`cycling.fit` and `running.fit` were generated with Garmin's official Python FIT
SDK 21.218.0. They contain no personal data. Tests read the committed binaries;
Python is not required to build or test either Rust application.

Both start at 2026-01-01 00:00:00 UTC, last 10 seconds, pause at 4 and resume at 6,
and have records every two seconds. The session/lap message timestamp is 90
seconds to catch use of save time as elapsed duration. Recorded speed is 2 m/s;
explicit enhanced speed is 3.123 m/s. Cadence includes a half-cycle fraction,
with one invalid cadence. Cycling includes zero and invalid power; running
power is invalid throughout. Session summaries are authored fixture values.
