# Agent instructions for slither_neuroevo

## Architecture and scope

Slither Neuroevolution is a browser-based neuroevolution sandbox with one
Rust-owned authoritative game. Rust owns world state, fixed-step scheduling,
sensors, heterogeneous neural inference, recurrent state, movement, food,
collisions, controllers, evolution, generation transitions, RNG/allocator state,
checkpoint construction, and frame packing. Node remains the thin HTTP,
WebSocket, static-file, archive-byte routing and SQLite-metadata interface.

The browser entry is `index.html`, UI is `src/main.ts`, packed-frame rendering
is `src/render.ts`, and styling is `styles.css`. Server startup is
`server/rustServer.ts`, with native coordination and persistence under
`server/rustEngine/`; Rust lives under `native/`. Migration-era `experimental*`
names in this retained production path are naming debt, not alternate runtimes.

The TypeScript simulator and Node inference pool were retired under the owner's
2026-10-10 plan. The complete combined implementation is frozen at branch
`codex/archive/ts-reference-and-rust-2026-10-10` and annotated tag
`archive/ts-reference-and-rust-2026-10-10`, commit
`abe20d695a4c8fbd5ae73f50973280c2c0801bbc`. See
`docs/decisions/0003-retire-typescript-runtime.md` for restoration instructions.
Do not reintroduce TS world/physics/sensing/evolution, neural execution, a
browser fallback, a second simulation loop, or a per-layer native bridge.

`package.json` and its lockfile define the Node toolchain; TypeScript, ESLint
and Vite use their root configs. `data/slither.db` and `server/config.toml`
are ignored owner state. `server/config.ts` creates default TOML only when
absent; never describe the generated file as tracked or rewrite it to migrate
removed settings.

The approved retirement is recorded in `docs/todo/typescript-runtime-retirement-plan.md`.
The prior Rust migration plan and factual implementation log remain historical
architecture/acceptance evidence. Deferred laptop/performance follow-ups stay
open. `project-recovery-plan.md`, `native_refactor_plan.md`, and `docs/todo/archive/`
are superseded; the former claim that the owner selected kernel-only Rust is false.
Durable decisions live under `docs/decisions/`.

## Runtime ordering and identities

Rust converts wall time and `simSpeed` into complete 60 Hz fixed steps.
Simulation speed never enlarges the physics delta. Overload may discard and
report wall debt; committed state never skips or partially commits a step.
All due controls use a stable observation boundary before movement, followed
by collision-only substeps. Delivered score-delta observations accumulate
between deliveries; baseline strategy probes do not consume that boundary.

Authoritative versioned RNG streams live in `native/src/engine/rng.rs`.
World, evolution, observer and baseline streams derive from the normalized
run seed; cosmetic rendering randomness must never advance them. Reset uses
the same seed with a new run ID; New Run uses entropy for a different seed.
Both become current only after durable run-start checkpoint commitment.

Exact replay applies only to matching source/RNG/snapshot versions, graph,
settings, backend build, target architecture, supported environment, completed
step count and ordered actions. Scalar/SSE2 comparisons use explicit numeric
tolerances; do not promise cross-build/platform bit-identical long-horizon replay.

## Protocol, client state and graph tooling

`server/wsHub.ts` owns socket connections and reliable/replaceable delivery;
`server/protocol.ts` and `src/protocol/` own shared wire types. Native external
routing uses `server/rustEngine/externalRouting.ts`. The browser reconnects
and waits for server frames after disconnect; it never owns a World.

`src/config.ts` contains browser setting drafts and presentation defaults.
Rust settings remain authoritative. Live updates use one atomic request and
settle from `settingsApplied`; reset-only settings and graphs use Apply and reset.
Welcome supplies seed, run/config identity, settings, sensors, serializer and
honest native inference mode. Preserve existing Protocol 2 fields, including
reserved epoch fields; native welcome pool/weight epochs remain null.

God Mode kill follows normal death; move translates the entire body within
bounds. UI logs use `godModeResult`. The status pill shows server, seed,
native backend and active worker count.

The only sensor layout is v3: `19 + 4 * bubbleBins`, bins at least eight.
Keep native sensors, `src/protocol/sensors.ts`, graph validation and UI aligned.
Pure graph schema/compiler/editor, stack builder and `parameterCounts.ts`
remain TypeScript. Graph execution belongs only to Rust. Ports are zero-based;
Split sizes sum to inputs, Concat ordering is explicit, outputs total two,
and graph keys/parameter counts are persistence compatibility boundaries.

`native/src/engine/frame_v1.rs` and `src/protocol/frame.ts` define frame-v1:
seven header floats; each alive snake has eight scalar floats plus body XY
pairs; then pellet count and five floats per pellet. Update native packing,
frame helpers, renderer, selection parsing and tests together. Keep compact
buffers rather than cloning native state into the browser.

## Native addon and validation

Production validates the complete coarse Rust bridge and independently
computed source/build identity. Missing/stale addons fail with build
instructions; there is no JS fallback. `--rust-workers` controls bounded
native calculation threads; defaults are normalized against available CPUs.
Do not pass raw defaults where a normalized config is required.

Standalone Dense/MLP/GRU/LSTM/RRU N-API exports are retired. Internal SIMD
dot products remain in `native/src/simd_kernels.rs`. Validate dimensions,
checked arithmetic, buffer lengths and aliasing at public native boundaries;
keep unsafe scopes narrow and document ranges/non-overlap. Supported targets
are x86_64 Windows MSVC and x86_64 Linux GNU; no WASM/non-x86_64 fallback.

```powershell
npm --prefix native run build
cargo test --manifest-path native/Cargo.toml --release
cargo fmt --manifest-path native/Cargo.toml -- --check
cargo clippy --manifest-path native/Cargo.toml -- -D warnings
```

If an npm shim fails, run the compiled napi-rs CLI from `native/`:
`node node_modules/@napi-rs/cli/dist/cli.js build --platform --release`.

Retain the five native source-identity fixtures and historical spawn,
movement, death, ambient and baseline-control evidence. Preserve bytes and
provenance; generators are archived. Live references to removed generators
must name the archive tag. Use frozen expectations, native regression tests,
scalar/SSE2 and multiworker continuation tests rather than reviving TS execution.

## Persistence and operations

Exact checkpoints are generation-boundary population saves, not mid-round
world snapshots. They capture evolved population, generation/step/run/config
identity, RNG/allocator state and zero recurrent state before spawn/sensing.
Immutable managed checkpoint-v3 files contain packed binary payloads with
raw or shuffled-Zstandard encoding. SQLite stores metadata, pointers, compact
history, graph/config records, Hall-of-Fame indexes and file references.

Export is one direct self-contained archive download; import uploads the
original File. Browser JS never parses or reconstructs populations. Keep
bounded TS-v2, legacy-gzip and legacy-JSON readers/conversion provenance.
Never delete or rewrite owner databases merely to migrate them. Newer builds
use approved labelled compatible-build recovery branches rather than claiming
exact replay across revisions. Startup selects the latest valid retained boundary.

## Loopback and trusted-LAN setup

```powershell
npm install
npm --prefix native run build
npm run server
npm run dev
```

Open Vite, normally `http://localhost:5173`; opening `index.html` directly does
not work. `play.bat` builds native and starts Rust/Vite with PID/log files;
`play.sh` builds native/client and starts Rust with static assets. Failed resume
preserves the database and health-only diagnostics.

Loopback is default. Trusted-home-LAN use supports `host`/`uiHost` bound to
an interface or `0.0.0.0`, and `publicWsUrl` for split-host routing. Preserve
launcher network discovery, usable UI/server/socket URLs and connectable HMR.
Origin admission/CORS are not authentication. This project has no authentication,
TLS or hardened public mode; never advertise router forwarding/untrusted exposure.

Important flags: `--rust-workers`, `--seed`, `--fresh`,
`--resume latest|sha256:ID`, `--db-path`, and LAN host/UI/socket overrides.
A configured seed with an existing resume requires `--fresh`. Removed backend,
Node-MT and tick flags reject with guidance. Neutral old TOML/environment values
warn and are ignored; active/malformed ones reject. UI rate remains at most 60 Hz.

## Tests and CI

`scripts/test-categories.ts` assigns every test exactly once to unit, component,
integration, system, acceptance, regression, performance or security. The
native-required additive overlay must load the source-identified addon and
exercise real Rust single/multiple-worker contracts; it cannot skip missing native.
Network skips require explicit `SLITHER_SKIP_NETWORK_TESTS=1` and a warning.

```powershell
node node_modules/tsx/dist/cli.mjs scripts/run-tests.ts all --reporter=dot
node node_modules/tsx/dist/cli.mjs scripts/run-tests.ts native-required --reporter=dot
node node_modules/typescript/bin/tsc -p tsconfig.json --pretty false
node node_modules/eslint/bin/eslint.js .
node node_modules/vite/bin/vite.js build
```

CI retains Ubuntu/Windows and Node 24/26, one production-native build per job,
source identity, all JS layers, native multiworker overlay, browser/type/lint,
and separate rustfmt/Clippy. Dependency guards verify retired paths are absent
and surviving client/server/metadata/tools cannot reach simulation execution.

### Verification cadence and usage efficiency

Use the cheapest validation that can disprove the current change, then widen
validation at meaningful checkpoints. Correctness gates remain mandatory; this
section controls how often expensive evidence is reproduced.

#### CI efficiency and checkpoint progression

CI validates cohesive checkpoints asynchronously. Do not treat completion of
the entire CI matrix as a blocking prerequisite for beginning the next local
implementation slice unless the current change is a release/cutover gate or a
failure could invalidate the architecture or correctness of the next work.

After pushing a cohesive checkpoint:

- Start or confirm the expected CI run, then continue useful local work while
  CI runs. Do not idle on `gh run watch`, repeatedly poll job state, or spend
  agent turns narrating normal CI progress.
- Check CI at natural work boundaries or when a notification/result is
  available. Do not repeatedly query unchanged running jobs.
- A failure in a test directly exercising changed code must be investigated and
  fixed before relying on that checkpoint.
- A failure in an unrelated existing test may receive one focused investigation
  to determine whether the checkpoint caused it.
- If an unrelated failure is clearly an infrastructure, runner-load, timing, or
  pre-existing test-flake problem, make at most one narrowly justified
  test-hygiene correction when the correction is obvious and low risk.
- Do not repeatedly restart the full CI matrix solely to chase unrelated
  timing flakes, overloaded-runner deadlines, transient dependency failures, or
  other infrastructure noise. Record the outstanding CI issue briefly and
  continue the migration.
- Do not widen timeouts merely to obtain a green run unless the original limit
  is demonstrably incompatible with the test's intended work. Preserve all
  correctness and liveness assertions.
- When several unrelated timing failures appear only under the full CI matrix,
  treat that as evidence of CI contention before treating each test as a new
  product defect.
- Prefer fixing CI scheduling once, such as serializing timing-sensitive
  integration files, over individually increasing many unrelated deadlines.
- Do not add durable evidence entries for ordinary CI progress, transient
  runner delays, dependency installation time, or resolved infrastructure
  flakes.
- Before pushing the next checkpoint, inspect the previous checkpoint's CI
  result. If it contains a plausible product regression, resolve that regression
  first. Otherwise continue without reproducing the full matrix locally.

The purpose of CI is to detect regressions, not to serialize development behind
every slow or flaky matrix job.

- During implementation, run focused tests for the files, invariants and known
  regressions affected by the current edit. Add a broader component/integration
  set only when the change crosses those boundaries.
- Do not run the complete Rust suite, complete JavaScript test matrix, full
  lint/type/build matrix, isolated checkpoint handoff and source-identity suite
  after every small edit. Run the broad local matrix before a cohesive feature
  checkpoint or push, after a change that can plausibly affect many subsystems,
  or when a formal stage gate requires it. CI is the normal cross-platform
  full-matrix confirmation for pushed checkpoints.
- A passing broad suite does not need to be rerun merely to give a reviewer an
  independent copy of the same result. Reviewers normally inspect the diff and
  run only focused tests needed to verify a concern they identify.
- Use Oxygen/Debian for target-platform behavior, Linux-specific changes,
  performance measurements and named stage/acceptance gates. Do not repeat a
  disposable Oxygen clone and full validation for an ordinary Windows-side
  micro-slice that has no Linux-specific behavior.
- The owner explicitly authorizes agents to use Oxygen for repository testing.
  This includes transferring project source, synthetic test fixtures and build
  artifacts; running builds, tests and benchmarks; and cleaning up task-owned
  test processes and files. Routine testing on Oxygen does not require repeated
  permission. Use the connection configuration available locally. Do not publish
  private SSH setup details, including addresses, account names, ports, key paths,
  credentials or connection commands, in tracked repository files.
- Reuse fresh evidence from the immediately preceding checkpoint when the
  current change cannot invalidate it. State the dependency instead of
  reproducing the same benchmark or compatibility run.
- Tooling, sandbox, shell, path, missing-build-artifact and permission failures
  are transient task notes once resolved. Preserve them in durable project
  documentation only when they reveal a real portability/product defect or
  change the implementation.
- Source/build identity checks should stay automated. Durable prose normally
  records the commit or source identity only when it is needed to identify the
  tested code; do not narrate canonical byte counts, path-byte counts, temp
  directory names or other bookkeeping that the automated check already
  proves.

### Temporary workspaces and retained evidence

- Treat `/tmp` as disposable scratch that may disappear on reboot. Do not use
  it as an archive or the sole location of evidence needed for a migration gate.
- Stop disposable processes and remove task-owned scratch, build artifacts and
  copied databases as soon as their validation slice no longer needs them.
  Verify the exact paths before cleanup; never sweep unrelated temporary files
  or another task's checkout.
- Keep reusable remote checkouts and fixtures in a clearly named persistent
  working directory. Give them the same cleanup lifecycle; moving files out of
  `/tmp` is not a reason to retain unused copies or build caches indefinitely.
- Save required benchmark reports and other retained evidence to a permanent
  project evidence location before cleaning the workspace. Retain the compact
  evidence needed to reproduce or assess a result, rather than abandoned
  binaries, clones or database copies.
- Check the actual filesystem's free space before a durability or loaded
  checkpoint run. Preserve the configured disk-admission reserve.

## Coding and documentation rules

- Preserve hot-path typed arrays and avoid per-frame allocation unless a
  measurement justifies it.
- Add TSDoc-style documentation for functions, classes, class fields, and
  module-level variables in `src/`, `server/`, scripts, and tests.
- Keep shared wire types under `src/protocol/` or `server/protocol.ts`; do not
  recreate browser-worker message surfaces.
- `README.md` is for users and QA. `AGENTS.md` is the developer reference.
  `docs/API-instructions.md` is the local external-client contract.
- Keep README slider names aligned with
  `src/protocol/settingDefinitions.ts`. Do not document removed v2 sensor or
  frame-delta controls.
- Use ordinary CommonMark with blank lines around lists and fenced blocks.
- Keep migration documentation compact. Prefer source comments, automated
  tests/fixtures, CI artifacts and the factual implementation log over a new
  prose evidence dossier for each implementation slice. Create a standalone
  evidence document only when it preserves information that is genuinely
  awkward to encode in tests or CI, such as a benchmark report, compatibility
  inventory, owner-data audit or a non-obvious cross-platform investigation.
- Existing verbose evidence files are historical records, not templates for
  future work. Do not expand or imitate them solely for consistency.
- Never commit `data/slither.db`, generated native binaries, PID/log files, or
  `server/config.toml`.
- Preserve Rust generation-best-score initialization before the first sensor pass.
- Treat `populationSlot`, snake-array index, visible snake ID, baseline-bot
  slot, and external controller ID as different identities.
