# Internal tools

Maintained developer utilities live here. Each tool should have a focused
command-line interface and may depend on reusable crates from `../crates`, but
production code must never depend on a tool.

Good candidates include FIT inspection, ride-sample analysis, and reproducible
dataset preprocessing. Exploratory notebooks and generated datasets should stay
outside production crates; do not commit personal ride data, credentials, model
artifacts, or large generated outputs.
