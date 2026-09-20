# Internal tools

Maintained developer utilities live here. Each tool should have a focused
command-line interface and may depend on reusable crates from `../crates`, but
production code must never depend on a tool.

Good candidates include FIT inspection, ride-sample analysis, and reproducible
dataset preprocessing. Exploratory notebooks and generated datasets should stay
outside production crates; do not commit personal ride data, credentials, model
artifacts, or large generated outputs.

## Intervals.icu API discovery

[`intervals-icu-discover`](intervals-icu-discover) performs one bounded,
read-only planned-workout request using an API key from the environment. It
prints a privacy-safe structure report and can optionally create a sanitized
fixture for manual review. See
[`docs/intervals-icu-discovery.md`](../docs/intervals-icu-discovery.md).
