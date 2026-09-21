# Contributing to TrainerPro

TrainerPro is a small project with deliberate product and architecture
boundaries. Discuss substantial features before implementing them; focused bug
fixes and documentation corrections can go directly to a pull request.

## Development setup

Install stable Rust, Node 20+, and the platform build tools. Then run:

```bash
npm install
npm run tauri:qa
```

TrainerPro QA uses a separate bundle identifier and data directory. Prefer the
simulated trainer and HRM; they support the same contracts and fault injection
as the physical-device paths.

## Before submitting

Run focused tests while developing. For changes spanning Rust and TypeScript,
finish with:

```bash
cargo test --workspace
npm run build
```

Check formatting before applying it repository-wide. If the baseline fails in
unrelated files, do not create formatting churn. Before committing, inspect
`git diff --check`, the changed-file list, and `git status`.

Never commit `dist/`, `target/`, credentials, personal activity data, or built
application bundles.

## Architecture guardrails

- Keep `tp-core` pure: no I/O, async, BLE, or Tauri dependencies.
- Keep trainer and heart-rate hardware behind `TrainerConnection` and
  `HeartRateConnection`; the simulator must be able to exercise the behavior.
- Extend the component that already owns a responsibility. Do not create
  parallel state channels or generic abstractions without multiple consumers,
  a real invariant, or a clear ownership boundary.
- Treat serialized Rust values and `frontend/ipc.ts` as one API.
- TPW is the canonical workout definition. ZWO, ERG, MRC, and provider formats
  are boundary adapters, not internal identity.

See [the architecture guide](docs/architecture.md) for ownership and flows,
[the behavior spec](docs/SPEC.md) for current requirements, and
[the TPW specification](docs/TPW.md) for the workout format.

## Adding a workout source

The existing library-source extension point consists of a backend adapter that
produces a `WorkoutDefinition` and a frontend descriptor in
`frontend/sources.ts`. Configuration uses the existing source values bag.

Provider schedule sync is a different lifecycle. Add a concrete integration
with explicit identity, ownership, retry, and reconciliation semantics; do not
turn the library-source registry into a generic connector framework.

## Documentation

Documentation is part of the product contract. Apply these rules whenever a
change affects it:

1. **State purpose and audience.** Every file under `docs/` begins with a
   one-line purpose and audience.
2. **Choose one category.** A document is product/feature-oriented (intent,
   behavior, scope, external contract) or technical/architecture-oriented
   (boundaries, flows, invariants, normative formats).
3. **Give each fact one home.** Link to the canonical explanation instead of
   copying it. `PRODUCT.md` owns the durable destination, `SPEC.md` current
   observable behavior, and `ROADMAP.md` unfinished sequencing.
4. **Prefer current truth.** Delete completed plans and superseded research;
   Git history is the archive.
5. **Let code own implementation detail.** Internal types, SQL, IPC payloads,
   protocol bytes, constants, and test matrices belong in code and tests unless
   they form a public or normative contract.
6. **Keep rationale proportional.** Preserve it only when it prevents a likely
   future mistake. Do not retain catalogs of rejected alternatives.
7. **Stay concise.** About 1,000 words is the default budget. A normative
   reference may be longer when every extra section is part of its contract.

A new feature document needs durable external knowledge or a distinct audience
that cannot be served by an existing document. Name it `docs/feature-<name>.md`.

## Releases

A `v*` tag triggers `.github/workflows/release.yml`, which builds unsigned
macOS and Windows packages and attaches them to a draft GitHub release. Signing
and notarization credentials remain maintainer responsibilities.
