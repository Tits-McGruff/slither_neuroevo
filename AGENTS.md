# Agent instructions for slither_neuroevo

Status: active developer instructions. Revised 2026-10-25 to align with the current evolved-lives/sensors/distillation plan and to remove temporary migration/qualification machinery from the permanent development model.

## Architecture and scope

Slither Neuroevolution is a browser-based neuroevolution sandbox with one Rust-owned authoritative game. Rust owns world state, fixed-step scheduling, sensors, heterogeneous neural inference, recurrent state, movement, food, collisions, controllers, evolution, generation transitions, RNG/allocator state, checkpoint construction and frame packing. Node remains the thin HTTP, WebSocket, static-file, archive-byte routing and SQLite-metadata interface. The browser remains presentation, configuration and control UI. PyRL is a separate trainer.

The browser entry is `index.html`; UI is `src/main.ts`; packed-frame rendering is `src/render.ts`; styling is `styles.css`. Server startup is `server/rustServer.ts`, with native coordination and persistence under `server/rustEngine/`; Rust lives under `native/`.

The TypeScript simulator and Node inference pool are retired. The archived combined implementation remains available through the existing archive branch/tag and `docs/decisions/0003-retire-typescript-runtime.md`. Do not reintroduce TypeScript world/physics/sensing/evolution, TypeScript or Node neural execution, a browser fallback, a second authoritative simulation loop or a per-layer native bridge.

`package.json` and its lockfile define the Node toolchain. TypeScript, ESLint and Vite use their root configs. `data/slither.db` and `server/config.toml` are owner state and stay untracked. `server/config.ts` creates default TOML only when absent.

Durable architectural decisions belong under `docs/decisions/`. Plans under `docs/todo/` describe active or historical project work and are subordinate to current code plus accepted decisions. The active `docs/todo/evolved-lives-sensors-distillation-plan.md` defines the feature behavior and current project/process direction. Historical implementation logs, benchmark receipts and qualification reports are historical records rather than standing development requirements.

## Development model

Prefer ordinary source control, ordinary tests and established libraries over project-specific registries, evidence systems or reconstruction frameworks.

Git is the source-provenance system. Experiments should normally identify code by a clean commit SHA. When experimental code differs from the active feature branch, use a temporary `experiment/...` branch or a deliberate WIP experiment commit. Use a base commit plus a patch only when committing the complete experimental state is genuinely inappropriate.

Do not copy tracked source trees into benchmark/evidence bundles. Do not create compressed source snapshots, source-reconstruction archives or branch-equivalence systems. Untracked experimental files that matter must be deliberately committed, preserved as a small explicit patch/fixture, or otherwise recorded as an actual artifact.

Routine generated experiment outputs belong in ignored local directories or CI artifacts. A small number of deliberately retained historical results may use a durable external artifact or GitHub Release asset. Git should retain only compact summaries, manifests and small regression fixtures that remain useful to current development.

Temporary experiment branches are development lanes. Merge or cherry-pick useful work, preserve any genuinely useful rejected result in a concise form, then delete obsolete branches and their dedicated automation. Do not make rejected historical experiments permanent CI obligations.

If Slither and PyRL require duplicated structural definitions, prefer at most one small shared contracts project for canonical graph/artifact/wire schemas and shared version/hash identifiers. Use an established schema/code-generation system where it materially helps. Keep simulation semantics, trainer semantics and product-specific validation in their owning repositories. Do not create a general provenance, experiment-management or simulation framework.

## Runtime ordering and identities

Rust converts wall time and `simSpeed` into complete 60 Hz fixed steps. Simulation speed never enlarges the physics delta. Overload may discard and report wall debt; committed state never skips or partially commits a step. All due controls use a stable observation boundary before movement, followed by collision-only substeps. Delivered score-delta observations accumulate between deliveries; baseline strategy probes do not consume that boundary.

Authoritative versioned RNG streams live in `native/src/engine/rng.rs`. World, evolution, observer and baseline streams derive from the normalized run seed. Cosmetic rendering randomness must never advance authoritative streams. Reset uses the same seed with a new run ID; New Run uses entropy for a different seed. Both become current only after durable run-start checkpoint commitment.

Exact replay is scoped to matching source/RNG/snapshot versions, graph, settings, backend build, target architecture, supported environment, completed step count and ordered actions. Scalar/SSE2 comparisons use explicit numeric tolerances. Do not promise cross-build/platform bit-identical long-horizon replay.

Keep these identities distinct throughout the codebase:

- population slot;
- snake-array index;
- internal physical snake ID;
- visible/frame snake ID;
- population brain handle;
- baseline-bot slot;
- external-controller identity;
- run/configuration/world incarnation;
- retained-brain origin.

A physical evolved life may be replaced while population slot, genome and population brain ownership remain unchanged.

## Protocol, client state and graph tooling

`server/wsHub.ts` owns socket connections and reliable/replaceable delivery. `server/protocol.ts` and `src/protocol/` own shared wire types. Native external routing uses `server/rustEngine/externalRouting.ts`. The browser reconnects and waits for server frames after disconnect; it never owns a World.

`src/config.ts` contains browser setting drafts and presentation defaults. Rust settings remain authoritative. Live updates use one atomic request and settle from authoritative replies; reset-only settings and graphs use Apply and reset. Welcome supplies seed, run/config identity, settings, sensors, serializer and the actual native inference mode.

Preserve Protocol 2 compatibility unless a deliberate protocol revision is made. God Mode kill follows ordinary death semantics; God Mode move translates the complete body within bounds.

V3 sensors remain the frozen compatibility layout. V4 Basic/Motion structural identities, profile/bin widths and ray-work settings are governed by the active feature plan and Rust canonical sensor metadata. Fresh defaults are Motion/B16 with baseline v2 stochastic at 1/60 s and SSE2, selected by Windows/Oxygen P0 real-time and query-quality qualification. P99 is reported, not a separate per-step deadline gate. Do not silently change recorded/imported settings.

Pure graph schema/compiler/editor, stack builder and parameter-count presentation may remain TypeScript. Graph execution belongs only to Rust. Ports are zero-based; Split sizes sum to inputs; Concat ordering is explicit; outputs total two; graph keys and parameter/state layouts are persistence/interoperability boundaries.

`native/src/engine/frame_v1.rs` and `src/protocol/frame.ts` define frame-v1. Update native packing, frame helpers, renderer, selection parsing and tests together. Keep compact buffers rather than cloning native state into the browser.

## Native addon and Rust safety

Production validates the coarse Rust bridge and a trustworthy build/source compatibility identity where that identity is required for safe loading. Missing or stale addons fail with build instructions; there is no JavaScript simulation fallback. `--rust-workers` controls bounded native calculation threads and must be normalized against available CPUs.

Standalone Dense/MLP/GRU/LSTM/RRU N-API execution exports are retired. Internal SIMD kernels remain ordinary Rust implementation details. Validate dimensions, checked arithmetic, buffer lengths and aliasing at public native boundaries. Keep unsafe scopes narrow and document actual safety invariants.

Supported production targets are x86_64 Windows MSVC and x86_64 Linux GNU unless the project deliberately adds another target.

Useful commands:

```powershell
npm --prefix native run build
cargo test --manifest-path native/Cargo.toml --release
cargo fmt --manifest-path native/Cargo.toml -- --check
cargo clippy --manifest-path native/Cargo.toml -- -D warnings
```

If an npm shim fails, the compiled napi-rs CLI may be run directly from `native/`.

Keep small native compatibility/source-identity fixtures only where they protect current loading, compatibility or a known regression. Historical generator infrastructure does not need to remain live merely to reproduce old evidence.

## Persistence and operations

Population import, export preparation and fresh-run construction share native resource admission at a complete held-step boundary. Charge the live source and successor/workspace requirements before background population allocation. Do not resume physics while a publication/import operation still owns the authoritative reservation. Release holds as soon as the operation no longer requires them.

Generation checkpoints are boundary population saves rather than mid-round world snapshots. They preserve the state required by the current checkpoint contract and its compatibility rules. SQLite stores metadata, pointers, compact history, graph/config records, Hall-of-Fame indexes and file references.

Checkpoint/archive implementation should be as simple as the actual atomicity, boundedness and compatibility requirements allow. Prefer standard archive/database/filesystem facilities over custom container parsers. Do not use runtime checkpoints to preserve source trees.

Export remains a direct archive/file operation and import consumes the original file. Browser JavaScript does not reconstruct population internals. Preserve bounded readers for supported historical formats. Never delete or rewrite owner databases merely to migrate them. Failed resume preserves owner data and provides diagnostic information.

Cross-build recovery must be labelled honestly. Do not imply exact continuation across incompatible source/schema/backend revisions.

## Distillation and saved artifacts

Teacher capture is diagnostic/training transport and never acquires external-controller authority. Rust owns committed teacher samples and candidate admission; Node stores/routes opaque bounded data; PyRL consumes/trains independently.

Runtime trainer checkpoints preserve trainer state, optimizer state, replay/sampler state, relevant RNG and immutable job identity. They do not preserve source trees. Use the simplest standard storage design that satisfies atomic restore and bounded-resource requirements.

Saved brain artifacts and shared Slither/PyRL contracts must carry enough structural identity to reject incompatible graph/sensor/deployment layouts. Numeric environment differences may remain provenance where the product intentionally permits them.

Large corpora, model collections, long traces, qualification studies and profiler outputs are generated artifacts. Do not commit them to normal Git history. Keep only selected small fixtures or summaries that protect current behavior.

## Loopback and trusted-LAN setup

Typical development startup:

```powershell
npm install
npm --prefix native run build
npm run server
npm run dev
```

Open the Vite URL, normally `http://localhost:5173`; opening `index.html` directly does not provide the application runtime. Preserve the existing launchers where useful.

Loopback is the default. Trusted-home-LAN use may bind server/UI to a local interface or `0.0.0.0` and may use `publicWsUrl` for split-host routing. Origin admission/CORS are not authentication. The project has no hardened public authentication/TLS mode; do not document router forwarding or untrusted public exposure as supported.

Do not commit or publish private remote-testing details such as addresses, usernames, ports, key paths, credentials or connection commands.

Important runtime flags and settings should remain documented in user-facing help/README when they are actually supported. Remove obsolete flags rather than maintaining compatibility machinery for dead configuration surfaces unless owner data requires a migration path.

## Tests

Tests should be discoverable by ordinary test-runner configuration, directories and/or filename conventions. Do not maintain a master file-by-file taxonomy such as a central registry that must be edited whenever an ordinary test is added.

Reasonable organization may include directories or suffixes for unit, integration, native-required, system/end-to-end and performance/qualification tests. The exact layout can follow the repository's tooling, but test discovery should be automatic.

Retain tests that protect current behavior, compatibility, numerical correctness, persistence, transactionality and known regressions. Delete tests whose only purpose is asserting completeness of a manual registry or preserving obsolete migration/evidence machinery.

Native-required tests must actually load the native addon when the behavior under test depends on Rust. They should fail clearly when a required addon is unavailable rather than silently exercising a substitute runtime.

During implementation, run focused tests for changed invariants first. Broaden to component/integration suites when the change crosses those boundaries. Run the full relevant local suite before a cohesive final checkpoint when warranted, rather than after every edit.

Typical ordinary checks may include:

```powershell
cargo test --manifest-path native/Cargo.toml --release
cargo fmt --manifest-path native/Cargo.toml -- --check
cargo clippy --manifest-path native/Cargo.toml -- -D warnings
node node_modules/typescript/bin/tsc -p tsconfig.json --pretty false
node node_modules/eslint/bin/eslint.js .
node node_modules/vite/bin/vite.js build
```

Use the repository's ordinary JS/TS test runner after test-layout cleanup. Do not recreate a central test catalogue merely to preserve the old command surface.

## CI and qualification

Routine CI answers whether current code builds and remains correct. Keep it reasonably fast.

Routine CI should cover the supported codepaths with a representative matrix rather than multiplying every operating-system, runtime-version and test-category combination unless a real compatibility requirement justifies it. Prefer one primary Node version plus an explicit compatibility job for additional supported versions where needed. Prefer one native build per job and reuse it within that job.

Routine CI may include:

- Rust tests;
- rustfmt and Clippy;
- TypeScript typecheck/lint/build;
- ordinary JS/TS unit/integration tests;
- Python lint/tests when PyRL is involved;
- native-addon smoke/load checks;
- a small number of cheap cross-language contract/roundtrip checks;
- dependency guards that prevent the retired simulation runtime from returning.

Expensive scientific qualification is separate. Long sensor-wall runs, soak tests, large learned cohorts, tactical ablations, maximum-capacity storage/replay tests, extended performance profiles, long live-distillation runs and historical experiment reproduction should use explicit qualification workflows, normally `workflow_dispatch`, scheduled qualification where useful, or release/default-selection gates.

Do not run historical-evidence reconstruction on every push. Do not keep one-off migration workflows indefinitely after the migration is complete. Do not require rejected experiment branches to stay green forever.

CI artifacts are appropriate for generated reports. A CI report does not need to become a Git commit.

A failed routine CI check that directly exercises changed code must be investigated. Treat clearly unrelated infrastructure/transient failures as infrastructure issues rather than product requirements. Do not inflate timeouts or add machinery merely to force a green matrix.

## Benchmarks, performance and retained results

Performance claims require measured evidence, but benchmark infrastructure should remain proportional to the decision being made.

For an optimization or default-selection experiment:

1. identify the tested source by Git commit/temporary experiment branch;
2. use a reproducible command or small benchmark harness;
3. record the relevant configuration and environment facts;
4. compare against an appropriate baseline;
5. keep the output as a CI/local artifact unless the result deserves deliberate long-term retention;
6. merge useful code or record the rejection, then retire temporary branch/workflow state.

Do not retain source snapshots inside benchmark results. Do not create permanent validators whose only job is repeatedly proving that an old benchmark bundle can reconstruct a historical working tree.

Resource measurements must avoid leaking unrelated host details. Publish only aggregate counters required to interpret the result. Keep process lists, usernames, hostnames, distribution inventories, credentials and other unrelated machine details local.

Use the configured Linux target host for Linux-specific behavior and named performance/release gates when needed. Routine testing on an already-authorized development host does not need repeated permission, but task-owned processes and scratch files should be cleaned up.

Temporary workspaces are disposable. Keep required durable artifacts in their intended external artifact location before cleanup. Do not use `/tmp` or build caches as the sole archive of a result that the project intends to preserve.

## Branches and history

The active feature branch may use short-lived experiment branches. After an experiment is integrated or rejected, remove obsolete branch-specific CI and metadata.

Before deleting or rewriting published feature history, preserve genuinely unique work and any required rollback backup. Do not rewrite `main`, unrelated historical branches or release tags casually. Never keep old payload-heavy history reachable through hidden recovery refs merely to satisfy an obsolete provenance mechanism.

When history has deliberately been rewritten, update operational cross-repository pins to the new commits while retaining old measured commit IDs only where they remain useful historical provenance. Avoid merging stale pre-rewrite feature branches back into cleaned history.

## Coding and documentation rules

- Preserve hot-path typed arrays and avoid per-frame/per-step allocation where practical. Optimize measured bottlenecks rather than adding complexity speculatively.
- Add documentation to public APIs, non-obvious invariants, unsafe code and complex algorithms. Do not require boilerplate TSDoc on every trivial local variable, field or test helper.
- Keep shared wire types under `src/protocol/`, `server/protocol.ts` or the selected shared-contract package. Do not duplicate protocol structures independently across projects.
- `README.md` is for users and QA. `AGENTS.md` is the concise developer/agent reference. `docs/API-instructions.md` is the local external-client contract.
- Keep README controls/settings aligned with the actual setting definitions. Remove documentation for retired controls.
- Use ordinary CommonMark with blank lines around lists and fenced blocks.
- Keep migration documentation compact. Git history, focused regression tests and current design docs are normally enough.
- Do not append ordinary CI/test counts or implementation diary entries to the active behavioral plan or `AGENTS.md`.
- Create standalone benchmark/compatibility documents only when the information remains useful and cannot be represented more simply by source, tests, an artifact and a short summary.
- Never commit `data/slither.db`, generated native binaries, PID/log files, `server/config.toml`, large generated qualification payloads or copied source archives.
- Preserve Rust generation-best-score initialization before the first sensor pass.
- Treat all runtime identities listed above as distinct.
- Avoid adding a new package, framework, registry or durable metadata layer when ordinary code plus an established library solves the problem.

## Handoff rule

When this revised `AGENTS.md` is supplied together with the revised evolved-lives/sensors/distillation plan, treat both as owner-authored replacements for their older feature-branch versions. Integrate them into the cleaned active feature history and any surviving related branch that genuinely needs the developer instructions. Do not preserve conflicting older copies merely for branch parity.

If an older implementation log, historical plan paragraph, benchmark README or migration test conflicts only on development process, evidence retention, CI cadence, experiment-branch handling, source provenance or test organization, these revised instructions and the revised plan take precedence. Product/runtime behavior and compatibility contracts remain governed by current source, accepted decisions and the revised behavioral plan.
