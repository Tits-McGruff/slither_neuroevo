# Evolved lives, long generations, sensor v4 and canonical-graph distillation

Status: active behavioral and architectural plan. Created 2026-10-11; revised 2026-10-25.

This revision preserves the settled product behavior of the previous plan while superseding the process, evidence, CI, experiment-branch, source-provenance, test-taxonomy and cross-repository contract practices that accumulated during implementation. Historical implementation logs, benchmark receipts, test counts and qualification reports remain historical records; they are not standing requirements for routine development or CI.

Source revision used for this update: `Tits-McGruff/slither_neuroevo` branch `evolved-lives-sensors-distillation`, plan blob `d721318ff827ebd38c38c7d9f38f1dbfc6348a05`, observed at feature head `c053992fd3af520232a9fa1cb49c8cc7fe5402f3`.

## 0. 2026-10-25 architecture and process revision

This section is authoritative over any conflicting process, provenance, branch-management, evidence-retention, testing or CI wording inherited from the previous plan. Behavioral/runtime compatibility requirements in later sections remain authoritative unless explicitly changed here.

### 0.1 What the plan is for

The plan defines product behavior, architectural ownership, compatibility boundaries, resource limits, interoperability contracts, implementation stages and the measurements required before selecting production defaults. It is not an implementation diary, a CI transcript, an experiment database or an archive of every failed optimization.

Do not append ordinary test counts, individual CI run results, repeated checkpoint receipts, per-experiment source inventories or long chronological implementation narratives to this plan. Important design decisions belong in concise design documentation. Benchmark and qualification outputs belong in benchmark/qualification artifacts. Git history already records implementation chronology.

Historical measurements cited by the previous plan remain useful historical context, but they do not create an obligation to reproduce every historical experiment on every future change.

### 0.2 Git is the source-provenance system

Experiments identify code through Git.

Preferred order:

1. Exact clean commit SHA.
2. A temporary `experiment/...` branch or WIP experiment commit when the experiment differs from the active feature branch.
3. A base commit plus an explicit patch only where committing the experimental state is genuinely inappropriate.

Do not archive tracked source trees into experiment bundles. Do not copy dirty working trees into ZIP, tar or compressed-source collections. Do not create source-snapshot reconstruction systems. A promoted experiment must make dirty state explicit rather than silently preserving it as copied files.

An ordinary patch must not be treated as capturing untracked files. When exact reconstruction matters, prefer committing the complete experiment state on a temporary branch.

### 0.3 Experiment outputs and evidence

Routine experiment outputs are ephemeral artifacts, not source-controlled documentation.

Use, as appropriate:

- CI artifacts for routine CI/qualification outputs;
- explicit benchmark output directories ignored by Git for local work;
- a durable external artifact store or GitHub Release asset for a small number of intentionally retained historical results;
- compact human-readable summaries and manifests in Git only when they remain useful to current development.

Do not put large generated corpora, model collections, profiles, run traces, complete study outputs or source snapshots into ordinary Git history.

The existing `qualification-evidence-v1-20261024` release assets remain archival records. Current source does not need a permanent framework whose purpose is repeatedly proving that those historical migrations remain reconstructable. Retain small regression fixtures only when they protect current behavior, compatibility or a known numerical counterexample.

### 0.4 Experiment branches are temporary lanes

The active feature branch is `evolved-lives-sensors-distillation` until this feature is merged or otherwise completed.

Related experimental branches are temporary development state, not permanent product subsystems:

- `evolved-lives-crowded-sensors`
- `evolved-lives-sensor-ray-reuse`

If a branch contains no unique useful work, remove it after confirming that fact. If a rejected experiment contains useful unique code, preserve the useful result as ordinary source, a concise patch/benchmark fixture or a deliberately retained archive branch, then remove permanent branch-specific CI and qualification machinery. Do not require rejected historical experiment branches to remain green forever.

Future pattern:

1. branch `experiment/...` from the active feature state;
2. implement and benchmark the alternative;
3. merge/cherry-pick useful work or record the result;
4. delete the obsolete experiment branch and its dedicated automation.

Do not create a branch registry, branch-equivalence framework or permanent replay system for temporary experiments.

### 0.5 One cross-project contract layer

Slither and PyRL currently duplicate structural definitions that must agree across Rust, Python and sometimes TypeScript. Extract those definitions into at most one small shared contracts project if doing so materially reduces duplication.

The shared contract layer may own:

- canonical graph schema;
- shared graph operation definitions;
- parameter/state layout descriptions;
- brain artifact format and identity/version rules;
- teacher/distillation wire schema;
- dataset manifest structures;
- shared format/version identifiers;
- canonical hashing/domain-separation identifiers;
- the small portion of source/build identity that is genuinely an interoperability contract.

Prefer an established schema/code-generation system where appropriate, such as Protobuf, FlatBuffers, Cap'n Proto or another suitable option selected from actual requirements. Generate/provide native Rust and Python bindings, and TypeScript bindings only for contracts TypeScript actually consumes.

Do not introduce PyO3/maturin merely because one consumer is Rust and another is Python. Use a Rust-backed Python extension only when Python must execute a single canonical Rust implementation. Structural schema sharing normally requires no FFI.

Keep Slither-specific and trainer-specific semantic validation in their owning projects. The shared contract package is not a generic simulation, ML, provenance or experiment-management framework.

### 0.6 Runtime checkpoints are separate from source provenance

A runtime trainer checkpoint may remain a single-file container when atomicity or portability makes that useful. Its implementation should use the simplest standard representation that satisfies the actual requirements.

Review PyRL `distillation_checkpoint.py`. Do not retain manual ZIP EOCD/ZIP64 parsing, custom archive admission or bespoke container validation merely because it already exists. If standard libraries plus bounded role validation are sufficient, use them. Suitable designs may include an atomically renamed directory plus manifest, a straightforward standard archive, `tar.zst` for sequential storage, or SQLite/an indexed store when genuine member-level indexed access is required.

Do not use checkpoint/archive containers to preserve source trees.

### 0.7 Test organization

Remove the manually maintained central test-file taxonomy. Organize tests using directory structure and/or filename conventions selected by ordinary test-runner globs/configuration.

Examples include `tests/unit/`, `tests/integration/`, `tests/system/`, `tests/e2e/`, or suffixes such as `.unit.test.ts` and `.integration.test.ts` where colocated tests are preferable.

Execution requirements such as “requires native addon” may use a directory, suffix or small explicit runner configuration. Adding an ordinary test must not require editing a master list of every test file. Delete tests whose sole purpose is verifying completeness of that master list.

Retain meaningful behavioral, compatibility, numerical and regression tests.

### 0.8 Routine CI versus qualification

Routine CI should be reasonably fast and answer whether the current code builds and remains correct.

Routine CI may include:

- Rust build/tests relevant to current product behavior;
- `rustfmt` and Clippy;
- TypeScript typecheck/lint/build;
- ordinary JS/TS unit and integration tests;
- Python lint and ordinary unit/integration tests;
- native-addon load/smoke validation;
- a small number of cheap Rust/Python/TypeScript interoperability roundtrips.

Do not run extensive scientific qualification on every push.

Move expensive work such as long sensor-wall runs, soak tests, large native cohorts, full tactical ablations, historical experiment reproduction, maximum-capacity qualification, extended performance measurements, long live-distillation runs and historical release-evidence reconstruction to explicit benchmark/qualification workflows triggered by `workflow_dispatch`, scheduled runs where useful, or release/default-selection qualification.

Retire one-off migration workflows such as `rewritten-feature-qualification` once their migration purpose has been completed. Historical-evidence integrity does not belong in routine acceptance testing unless current product behavior genuinely depends on that old artifact.

CI artifacts are appropriate for run outputs. Routine CI output must not automatically become permanent repository state.

### 0.9 Verification cadence

During implementation, run the cheapest relevant check that can disprove the current change. Do not rerun the entire Rust/JS/Python/qualification matrix after every small edit.

At a cohesive final checkpoint:

- run the affected unit/integration suites;
- run Rust format/Clippy if Rust changed;
- run TypeScript type/lint/build if TS changed;
- run Python lint/tests if Python changed;
- run a small cross-language interoperability set when shared contracts changed;
- run the complete ordinary test suites once.

Run expensive qualification only when the change actually affects the property being qualified or when making a release/default-selection decision. Do not create a new evidence bundle merely to prove that a cleanup occurred.

### 0.10 Keep legitimate custom code custom

Do not replace custom code merely because a library exists. The following remain legitimate project-specific code unless a concrete problem is identified:

- deterministic RNG compatibility with the existing TypeScript/Rust state contract;
- authoritative Slither simulation and transaction logic;
- sensor semantics and collision/world geometry;
- evolution/lifecycle logic;
- compact native inference where measured workload justifies it;
- current SSE kernels;
- PyRL training objectives, sampling and sequence semantics;
- game-specific semantic validation and compatibility rules.

The existing SSE path has measured value. Do not spend this architectural cleanup implementing AVX2. Inference is not the dominant current bottleneck.

Sensor and serialization performance work is a separate task. Existing measurements indicate sensing is a major cost. A later profiling/performance pass may investigate immutable sensor-phase snapshots, safe Rayon parallelism, independent worker output buffers, spatial-index contention, allocations/copies, hashing, Rust/JS boundary overhead, persistence locking and actual serialization CPU cost. Do not opportunistically redesign these systems during the infrastructure cleanup.

### 0.11 Simplification criterion

Before adding a package, framework, registry, compatibility layer or durable metadata system, ask whether ordinary code plus an established library solves the problem more simply.

This cleanup succeeds when Slither and PyRL contain fewer special-purpose mechanisms, less permanent experiment infrastructure, less duplicated cross-language structure and less routine qualification work. One small shared contracts project is the maximum new top-level architectural component expected.

## Current implementation state

Stages A-C and their core duration, life-accounting, respawn, identity, RNG and persistence behavior are substantially implemented. V4 sensors, baseline v2, canonical PyRL training, committed teacher capture, distillation transport/jobs, saved-brain/library support and inspector functionality are also substantially implemented. Their detailed historical test counts and experiment receipts are deliberately omitted here.

The previous plan accumulated many measurements while investigating sensor quality, performance, storage capacity, training maturity and integrated contention. Those measurements remain historical inputs to engineering decisions but are no longer copied into the canonical plan as an ever-growing implementation diary.

Remaining release/default-selection work should be restricted to unresolved product decisions, particularly representative sensor/profile/ray-limit selection, representative policy quality/maturity where required, sustained teacher/trainer throughput, admitted large-workload/resource ceilings, crowded/saturated geometry where it affects production settings, and the final integrated performance/soak checks that are genuinely necessary before promoting defaults. These are explicit qualification jobs, not routine per-push CI.

The behavioral contracts below remain the feature specification. Values explicitly marked as qualification gates require measurement before production-default selection. The process rules in Section 0 govern how those measurements are run and retained.

## 1. Architectural boundaries and current systems

Rust owns the sole authoritative simulation, fixed-step scheduling, sensors, heterogeneous graph inference, recurrent state, movement, food, collisions, controllers, evolution, RNG/allocators, checkpoints and frame packing. Node remains the HTTP/WebSocket/static-file/archive-byte/SQLite-metadata layer. The browser remains presentation, control and configuration-draft tooling. PyRL remains the trainer.

Do not restore the retired TypeScript world/sensors/neural execution, Node inference pool, browser fallback, second authoritative loop or per-layer native bridge calls.

The authoritative step remains ordered command service -> prefix accounting/spawn -> shared observation/control -> movement/food/collision/effects -> generation guard -> either nonterminal external replacement/delivery/publication or evolution/checkpoint/metadata/successor transaction.

Respawn belongs at the established pre-control newborn boundary. Life-end accounting belongs in the physical transaction. Manual finish belongs in the existing post-physics generation guard.

Important ownership distinctions remain:

- population slot, lineage, genome and population brain ownership persist across evolved physical lives;
- physical IDs, frame IDs, array indexes and population slots are distinct;
- recurrent state publication is transactional;
- external leases/routes are separate from evolved lifecycle semantics;
- frame-v1 remains compact and diagnostics stay on demand;
- TypeScript graph/settings code is schema/compiler/editor/presentation tooling only;
- persistence preserves immutable files, transactional pointers and bounded readers.

Current hard/compatibility limits include evolved population <=300, baseline slots <=120, body-point storage 1,000,000, pellet storage 250,000, graph tensor width 512 and packed parameters <=1,000,000 per graph. Protocol 2 and frame-v1 remain compatibility boundaries. Checkpoint-v3 remains a pre-spawn generation-boundary durability format, not an exact running-world snapshot. Exact replay is scoped to matching contracts/build/environment and completed ordered actions; cross-platform/build long-horizon bit identity is not promised.

## 2. Sandbox principles and defaults

Engine correctness means deterministic/versioned transitions, coherent ownership, explicit RNG, transactional failure, bounded memory/work, valid persistence and compatibility. It does not mean removing exploitable evolutionary incentives.

Cheap death, corpse farming, accumulated resources, repeated best-life attempts, cross-life memory, stochastic teachers and imperfect sensing may be selectable experiments or observable consequences.

Default layers remain:

| Situation | Rule |
|---|---|
| Recorded checkpoint/import | Preserve recorded contracts/settings; only explicit provenance-labelled compatibility handling. |
| Reset/New Run from selected configuration | Preserve configuration. Reset repeats seed; New Run selects new seed. Neither silently changes sensor/teacher. |
| New configuration using current defaults | v4 Motion, 16 bins, baseline v2 stochastic, respawn off, 240 seconds once those defaults have passed their required release qualification. |

Recorded/imported settings take precedence over fresh defaults. Browser drafts never override authoritative welcome values. Frozen v3/v1 compatibility remains selectable.

Confirmed product choices remain:

- selectable v3 and v4 sensors;
- Reset/Cumulative/Best Life point assessment;
- Cumulative Net/Gross earned, Net default;
- Cumulative Life-local/Carried wallet, Life-local default;
- non-point fitness aggregation independently selectable, Generation totals default with respawn;
- Per-slot/World respawn RNG, Per-slot default;
- recurrent Reset/Preserve, Reset default;
- Normal/None/Subtract replacement-mass corpse rules;
- Fail/Discard-new-excess pellet overflow, with explicit respawn preset selecting Discard;
- separate PyRL/PyTorch CUDA trainer using the canonical graph;
- founders, DAgger and exact mid-generation recovery remain future projects.

## 3. Stage A - duration parsing and manual completion

Authoritative `generationSeconds` remains numeric seconds with accepted range 8..604,800 and default 240.

The duration text grammar accepts plain unsigned decimal seconds; one to three integer `h`/`m`/`s` terms in any order without duplicate units; spaces/tabs and at most one comma separator; `M:SS`; and `H:MM:SS`. Input is bounded to 128 characters with checked accumulation. Reject fractions, signs, exponent/nonfinite notation, arithmetic expressions, garbage, duplicate units, malformed colon fields, doubled/trailing commas and overflow. Invalid drafts block Apply/reset rather than silently clamping.

Manual completion uses joined-UI `finishGeneration { requestId, expectedRunId, expectedGeneration }` through Protocol 2 validation and the coarse Rust command queue. It is idempotent and generation-scoped, completes the next eligible authoritative step normally, evaluates after physics/effects/life accounting, uses `GenerationTransitionReason::Manual`, does not bypass delivery/persistence barriers, rejects stale requests and cannot finish the successor generation. Precedence remains Manual -> Duration -> early-end rule.

Persist completion reason, actual elapsed time, configured duration and completed step. No pause subsystem is introduced.

Tests protect parser boundaries, long-duration completion, repeated/stale requests, coincident terminal conditions, delivery/persistence failures and ordered replay. Long-duration clock verification is a release/behavioral test, not a per-push multi-hour CI job.

## 4. Stage B - population-slot life state and economic ledger

`EvolvedGenerationState` owns one dense `EvolvedSlotRuntime` per population slot. Slot/genome/brain ownership persists across lives; physical identity changes on successful replacement.

Maintain lifecycle state, current-life economics, generation totals, assessment values, best-point-life tuple and bounded diagnostics. Lifecycle states remain Alive, Waiting, PendingPlacement, Exhausted and DeadNoRespawn. Initial life ordinal is 1 and increments only on successful spawn.

Required ledger identities remain:

```text
lifeBoostSpent = lifeBoostSpentFromLifeCredits + lifeBoostSpentFromPriorBank
lifeNetPoints = lifeGrossEarned - lifeBoostSpentFromLifeCredits
generationNetEarned = generationGrossEarned - generationBoostSpent
```

Track life gross earned, actual boost spent, spend attribution to current-life credits/prior bank, life-local wallet/net points, prior-bank remainder, generation gross/spent/net, assessment points and physical `SnakeState.points`. Preserve finite nonnegative balances and actual movement burn chronology.

Point assessment modes remain:

- Reset: current/latest life's net points;
- Cumulative Net: generation credits minus actual boost expenditure;
- Cumulative Gross: all generation credits regardless of boost spending;
- Best Life: highest net life-point peak, with earlier life winning ties.

Life-local wallet is default. Carried mode inherits net generation bank. Gross score never becomes spendable capital. Best-life peak survives later spending.

Non-point fitness bundle modes remain Generation totals, Follow score and Legacy terminal. Growth remains net retained growth `max(final_length - starting_length, 0)` per completed life plus current unfinished growth.

A source-live snake receives exactly one alive award per step. A successfully replaced newborn receives exactly one newborn award. Waiting/failed/exhausted slots receive none. A replacement must never receive both the dead-life and newborn award for the same step.

## 5. Stage C - evolved replacement, transient state and bounded placement

Population slot, lineage, weights and population brain ownership persist. Every successful life receives a new internal physical ID and available frame-v1 ID. World records are replaced in place rather than appended as historical lives.

A fresh physical life initializes collision-safe starting body/state, movement state, life counters, selected wallet and first-delivery state. Delivered-wallet boundary starts at the new wallet so inherited capital is not reported as new earnings. Recurrent Reset uses the existing reset-aware inference path; Preserve keeps recurrence while still forcing a fresh control evaluation. Neither preserves stale motor actions.

Capture a bounded life-end record before collision scratch disappears. Retain at most population-sized transaction records and a recent-event ring of 256; do not persist every life.

Replacement clears old pellet ownership while preserving pellet value/kind/color, releases retired bodies after final accounting, preserves population brain owner/handle, invalidates physical-selection/contact/visualization state and signals lifecycle boundaries to capture/shadows. Historical IDs are values, never pointers.

Respawn delay remains 0..3,600 seconds with Fixed or Uniform `[0.5D,1.5D]`. Maximum lives includes the initial life; zero means unlimited. Failed placement consumes no life and no successful-respawn penalty.

Placement remains bounded and transactional. Default internal limits remain eight attempted slots/step, 256 candidates/slot, 2,048 candidates/batch, 250,000 geometry checks/batch and 0.25 simulated-second unsuccessful retry interval unless later explicitly revised through product work. Order due attempts by deadline step, last-attempt step and slot. Failed attempts advance the selected RNG by consumed draws. Allocate IDs only after a staged successful placement. Congestion remains pending rather than faulting or silently exhausting lives.

## 6. Ordering and RNG contracts

Preserve existing physical ordering where physical identity is the existing tie-breaker. Do not globally replace physical IDs with population slots.

Stable population slot is used for evolution/accounting aggregates, evolved respawn attempt order and other explicitly slot-owned work. V4 observation quotas use stable observation-owner identities. Teacher streams remain baseline-slot/life scoped.

Default evolved respawn RNG mode is Per-slot. Derive streams centrally from:

```text
evolved-respawn/v1/generation/<decimal-u64>/slot/<decimal-u32>
```

Use existing labelled seed derivation over the full generation identity. Do not use a numeric seed hash that truncates the generation identity to Uint32. Same-seed Reset repeats derivation. Failed attempts preserve their consumed RNG continuation. Respawn never advances evolution RNG. World RNG remains world-owned for corpse/effect behavior where specified.

## 7. Fitness, ecology and generation completion

Legacy default fitness remains:

```text
F = 0.70 * age
  + 80 * food_value_eaten
  + 100 * max(body_length - start_length, 0)
  + 400 * kills
  + 42 * clamp(ln(1 + points) / ln(1 + maximum_points), 0, 1)
  + top_bonus
```

Top bonus remains 600 for points within `1e-6` of the maximum, without granting it automatically when all points are zero. Respawn fitness substitutes selected assessment points and selected non-point tuple. Optional life penalty remains 0..10,000 and can apply to Deaths or Successful respawns. Finite negative fitness remains valid.

Absolute duration scoring remains default. Optional 240-second normalized fitness scales extensive survival/food/growth/kills by `240/configured_duration` while leaving normalized points/top bonus/penalties and gameplay economics unchanged. Manual completion uses configured duration for this scaling.

Corpse modes remain Normal, None and Subtract replacement mass. Final exhausted death uses normal rules because no replacement follows. Pellet overflow remains Fail by default; the respawn preset may explicitly select Discard-new-excess. Discard preserves existing pellets, consumes defined draws in canonical order for attempted effects, allocates IDs only for retained pellets and records discarded count/value.

Automatic early completion remains legacy physical threshold when respawn is off, all-slots-exhausted for finite respawn, and duration/manual only for unlimited respawn. Physical-alive mode and Duration-only mode remain explicit alternatives. Pending placement congestion is not extinction.

## 8. History, HoF and long-generation durability

Persistent history stays bounded. Preserve existing compact fitness summaries and the fixed-size versioned experiment sidecar/HoF assessment records required by current product behavior. Do not turn those records into per-life/per-step logs.

HoF still selects the best-fitness genome. Versioned HoF records retain the assessment/bundle and selected-life semantics required to present Reset/Cumulative/Best Life correctly. Old record meanings are never silently reinterpreted.

Boundary durability remains the implemented recovery scope. Optional periodic counters/digests are diagnostics, not exact world resume. Exact mid-round recovery remains a separate future project requiring full world/brain/lifecycle/RNG/controller/scheduler state.

Source/schema recovery uses Git commit identity and explicit migration adapters/patches. A temporary recovery branch may be used while doing migration work, but do not create permanent source-recovery branches merely to retain historical source bytes.

## 9. Stage D - v3 audit and v4 sensor contract

V3 remains frozen compatibility. V4 structural identity includes version/profile, width, exact labels/offsets, channel meanings/order/geometry, normalization revision/constants and missing/validity encodings. Numeric environment settings such as world/range/food/cost/detail limits remain run settings/provenance rather than structural identity.

V4 scalar prefix remains 24 values:

`heading_sin`, `heading_cos`, `tactical_size`, `radius_norm`, `speed_norm`, `spendable_points_norm`, `boost_seconds_norm`, `boost_cost_norm`, `boost_eligible`, `actual_boost`, `held_boost_request`, `held_turn`, `previous_applied_turn`, `angular_velocity_norm`, `speed_change_norm`, `delivered_wallet_delta_norm`, `current_life_points_norm`, `assessment_points_norm`, `life_age_norm`, `generation_remaining_fraction`, `life_budget_used_norm`, `first_sample_of_life`, `pellet_query_saturated`, `body_or_ray_query_saturated`.

Profiles remain:

- Basic: near food, far food, other-body sector clearance, body capsule-ray clearance, wall-ray clearance, other-head clearance; width `24 + 6B`.
- Motion: Basic plus nearest-head presence/bearing/relative length/heading/relative velocity, projected-collision urgency and head density; width `24 + 15B`.

Bins remain 8..32. Motion/B32 is the current 504-input ceiling case under tensor width 512. Structural profile/bin mismatch rejects even when total widths happen to match.

Normalization remains explicit and versioned. The previous plan's candidate transforms for tactical size, wallet, life/assessment points, boost time/cost, age, delivered-wallet delta, angular velocity, speed change, relative length/velocity, collision urgency and density remain the starting contract. Any pre-release change updates the structural revision; released semantics are never silently retuned.

`first_sample_of_life` remains +1 until actual internal consumption or matching external acceptance, then -1. Probes/inspector/shadows do not clear it. Rejected delivery preserves it.

Ray work remains bounded and fair over canonical observation owners. `RayWorkLimits` must admit positive minimum <= per-observation cap and prove `maximum_due_observations * minimum <= batch_budget`. Workers do not redistribute by finish order. Unknown/incomplete rays remain conservative and validity-marked.

Production ray/profile defaults must be selected from representative quality and performance evidence, including active consumer classes, rather than by theoretical limits alone. Qualification candidates may include the existing cap/batch ranges, but this is a manual/release benchmark task, not routine CI.

V4 default promotion requires P0 qualification on both Windows and Oxygen. P1 remains a required representative overload/stress measurement: record throughput, latency/real-time factor and resource behavior, and verify that degradation is controlled rather than pathological. P1 real-time performance is not a hard promotion gate unless measurements establish that P1 represents an ordinary expected production workload rather than an intentionally heavier case.

Structural graph/sensor descriptions shared with PyRL belong in the shared contracts layer where practical. Slither-specific geometry and semantic checks remain in Slither.

## 10. Stage E - baseline v2 and tactical evaluation

Baseline v1 remains frozen compatibility. Baseline v2 consumes decoded v4 observations only for action selection and does not gain undeclared privileged world reads.

V2 retains motion/head-on/crossing risk, capsule routes, pursuit/escape, wall openings, food-trap discounting, crowd pressure, commitment/hysteresis and economic safe boost. Deterministic mode avoids strategy RNG; Stochastic mode uses an isolated behavior RNG separate from lifecycle RNG. V1 streams/behavior remain unchanged.

`baselineBots.controlDt` uses the shared due algorithm. Teacher/student deployment cadence must match; first-life evaluation is immediate. Capture never changes cadence.

Tactical fixtures and representative batches are qualification tools. Keep useful fixture generators and benchmark drivers, but do not preserve every historical run as Git payload or run the full tactical matrix on every push.

## 11. Stage F - canonical graph training in PyRL

PyRL remains a separate trainer. Distillation mode uses PyTorch/CUDA over the canonical deployed graph rather than introducing Rust autodiff/BPTT or substituting the existing PPO `PolicyValueNet`.

The canonical manifest describes graph spec/key, layout/equation revision/digest, topological order, resolved ports, node widths/layers, parameter blocks/offsets/lengths, recurrent layouts, output order, totals, structural sensor identity and deployment cadence/output mapping.

PyRL owns one flat Float32 parameter tensor and constructs differentiable views according to the canonical manifest. It does not independently invent parameter packing/order. Custom Dense/MLP, GRU, LSTM, RRU, Split, Concat, branch/fan-out and output semantics remain exact to the deployed Rust graph.

Method-v1 loss remains:

```text
Lturn = (t-a)^2
z = (b-0.35)/0.10
Lboost = BCEWithLogits(z,y)
L = eventWeight * (Lturn + Lboost)
```

Defaults remain Adam 3e-4, clip 1.0, batch 32 sequence windows, TBPTT 64, burn-in 32, Float32 unless explicitly changed through product work. State advances on deployment cadence, Reset/Preserve follows declared life boundaries, gaps terminate continuity and padding never advances recurrence.

The Rust whole-graph offline runner remains appropriate for parity/deployment validation. Preserve operation/DAG/recurrent parity and the optimizer-update -> export -> ordinary Rust validation/inference -> exact-byte reimport roundtrip.

Move the structural canonical-graph schema and shared operation/layout definitions into the shared contracts layer where this eliminates Rust/Python/TypeScript duplication. Do not force game-specific semantic or numerical validation into the schema package.

## 12. Stage G - committed teacher capture and bounded transport

Capture the exact observation already constructed by authoritative due control plus committed action/outcome metadata. Publish only after complete transaction success. Failed steps produce no samples and capture does not resample or consume an additional delivery marker.

Teacher samples retain run/config/sensor/teacher/cadence identity, generation/step, baseline slot/physical life/ordinal, Float32 vector, applied actions, strategy/life stage, hazard/saturation strata, sample sequence and boundary reasons. Chunk streams at at most 256 observations and seal at life/contract/capture/time boundaries as previously specified. Chunk boundaries alone do not imply recurrent reset.

Transport remains dedicated bounded diagnostic HTTP, separate from external controller leases. Rust owns committed sample encoding and boundary semantics; Node stores/routes opaque bytes and bounded metadata; PyRL consumes them. The authoritative simulation never waits on trainer/storage.

Dataset descriptors retain immutable identity, sequence/cursor, byte count/hash and watermarks required to distinguish produced, stored, retained and settled gaps. Missing data is never fabricated. Trainer restart persists its cursor/snapshot/sampler state.

Move the structural teacher/distillation wire schema and dataset manifest structure into the shared contracts project where appropriate. Keep chronology, life-boundary, committed-world and game-semantic checks in their owning code.

Do not maintain independent hand-written structural encoders/decoders in Rust and Python when schema generation can safely remove that duplication.

## 13. Stage H - shadow evaluation and durable jobs

At most four explicitly requested shadows remain empty-by-default. A shadow evaluates the same committed teacher vector with separate recurrence/cadence/life state and never publishes action, fitness or observation authority.

Trainer jobs own independent Python/NumPy/Torch RNG and bounded CPU/CUDA resources. They do not mutate world/baseline/evolution RNG, ordering, physics or live controlling weights.

`DistillationJobV1` remains a separate checkpoint owner from PPO. Persist graph/sensor/deployment/teacher/method identities, hyperparameters/loss revision, canonical weights, optimizer, trainer RNG, counters, immutable dataset/replay state, sampler/cursors/recurrence, validation/online metrics and numerical environment required for real restart.

Checkpoint implementation must be simplified according to Section 0.6. The requirement is an atomic, bounded, correctly validated trainer snapshot. The requirement is not a custom ZIP implementation. Do not store source-tree snapshots in job checkpoints; record Git source identity and explicit experimental patch/commit state instead.

Candidate artifacts remain ordinary immutable deployable brain artifacts validated independently by Rust.

## 14. Stage I - retained origins, saved brains and portable deployment

Managed retained neural objects remain shared beneath HoF/saved-brain consumers, with bounded catalog/GC/export leases. Runtime origin remains immutable metadata associated with retained neural state, not a per-step durable lookup.

Saved records retain graph/key/weights, structural sensor/cadence and relevant provenance/metrics. Distilled brains are saved-brain artifacts, not HoF winners. Candidates do not automatically populate the library.

Portable `.slither-brain` remains a bounded versioned artifact containing the graph/spec/digest, sensor/deployment contract, canonical Float32 weights/counts/encoding/digests and compact provenance. Rust independently recompiles/validates finite values, graph layout, width/count and digest. Browser/Node route bytes rather than interpreting trained models.

The structural brain-artifact format, identity/version fields and canonical hashing belong in the shared contracts layer where practical. Runtime spawn semantics, world identity, safe placement and catalog behavior remain Slither-specific.

## 15. Stage J - inspector and capacity qualification

Inspector remains opt-in, on-demand and bounded to one selected target/replaceable diagnostic stream. It shows real delivered or explicitly labelled probe data and never pretends held controls are fresh observations. Probes do not alter delivery markers, authoritative RNG or quota cursor.

Large-graph capacity remains an admission problem. Motion/B32 with 494,306 parameters per brain and a 300-genome population can exceed the current production checkpoint envelope. Candidate envelope changes must be measured before promotion. Do not silently raise limits or reduce graph dimensions until bytes happen to fit.

Resource qualification must cover the actual overlapping owners that matter: running/successor populations, checkpoint workspace, HoF/saved objects, archive/import, native scratch, Node spool, trainer replay/checkpoints, shadows and inspector where relevant.

Performance and soak qualification is explicit release/default-selection work. Run representative P0/P1 and necessary large/crowded cases, measure step percentiles, wall debt/throughput, warmed allocation/capacity, queue work, RSS/VRAM and checkpoint peaks. Six-hour or maximum-capacity runs are never ordinary per-push CI.

Use profiling to drive future optimization. Current evidence says sensing/serialization/world work dominate enough that further SIMD expansion is low priority. Preserve the measured SSE path and defer AVX2 unless future profiling changes that conclusion.

## 16. Persistence/version rollout and external-controller safety

Preserve Protocol 2 and frame-v1 while adding versioned capabilities. Explicitly version state/accounting, settings/hash, RNG bundles, sensor structural contract, baseline v2, compact experiment/HoF records, graph/sample/transport/job/candidate contracts, saved-brain objects and archive roles where required by actual compatibility.

Checkpoint-v3 remains the boundary container/purpose with versioned contained schemas. Readers dispatch by recorded version. Legacy conversion remains bounded and v3-specific; do not arbitrarily pad v3 brains into v4.

External controllers retain their lease/token/receipt/reclaim/input-hold/takeover/generation-reassignment semantics. Evolved respawn may share geometry/storage/reset helpers but never external lease semantics. Distillation transport cannot masquerade as an external controller.

Compatibility code should preserve old runtime data formats that users actually need. It should not preserve obsolete experiment-management machinery merely because historical qualification once used it.

## 17. Ordered implementation and acceptance matrix

| Stage | Deliverable | Dependency/gate |
|---|---|---|
| A | Duration/manual finish | Existing terminal transaction and parser/clock behavior |
| B | Life/economic/death accounting | Legacy one-life equivalence |
| C | Respawn identity/reset/placement | Accounting and transactional mutation proofs |
| Following B/C | RNG/scoring/ecology/termination/history | Selectable rules and bounded durable records |
| D | v4 prototype/default selection | Stable life/economic meanings, fair budgets, representative quality/performance |
| E | baseline v2/tactics | Shared qualified observation model |
| F | canonical manifest/PyTorch parity | Shared graph contracts; may develop in parallel once stable |
| G | capture/transport | Commit-safe samples and cadence/teacher identity |
| H | shadows/training/jobs | Manifest parity and bounded datasets |
| I | library/artifacts/spawn | Qualified candidates and managed retained objects |
| J | inspector/integrated capacity | Incremental diagnostics; manual/release integrated capacity/performance/soak |
| Future | founders/DAgger/exact recovery | Separate explicit projects |

Required behavioral coverage remains focused on:

- identity across physical replacement;
- transactional rollback and no partial state publication;
- bounded storage and owner cleanup;
- all ledger/scoring/wallet/bundle modes;
- safe/fair bounded placement and RNG continuation;
- corpse/overflow/ecology behavior;
- duration/manual/automatic completion;
- sensor layout/geometry/normalization/saturation/fairness/compatibility;
- baseline declared-input behavior and RNG isolation;
- canonical graph operation/recurrent/training parity;
- committed capture and transport gaps/leases/reconnect semantics;
- shadow/job restart and world-isolation semantics;
- legacy/current persistence compatibility;
- matching-worker-count authority/observation equivalence within the declared numerical contract.

Organize JS/TS tests by directory or filename convention and select them through ordinary Vitest configuration/globs. Do not maintain `scripts/test-categories.ts` as a master inventory of individual files. Native-required tests may have a simple native naming/directory convention and must load the actual built addon where appropriate.

Keep only small fixtures that protect current behavior/protocol compatibility or important numerical counterexamples. Historical source snapshots and old experiment payloads are not fixtures.

Focused tests are used during implementation. Routine CI runs the ordinary build/unit/integration/lint/type/format checks plus cheap interoperability. Expensive performance, soak, historical-reproduction and broad qualification jobs are manual, scheduled or release/default-selection workflows and run only when relevant. Do not create durable evidence bundles for ordinary progress.

## 18. Concrete settings surface

The existing setting surface remains unless a later product decision explicitly changes it. Stable wire enums and authoritative validation remain required.

### Generation, evolved lives and fitness

| Path | UI label | Type/range/options | Default | Apply |
|---|---|---|---|---|
| `generationSeconds` | Generation duration | 8..604800; duration text | 240 | reset/world |
| `observer.earlyEndMode` | Early completion | Automatic / Physical alive / Duration only | Automatic | reset/world |
| `observer.earlyEndMinSeconds` | Earliest early completion | existing 0..50 | 8 | existing live/checkpointed |
| `observer.earlyEndAliveThreshold` | Alive threshold | existing 1..25 | 2 | existing live/checkpointed |
| `evolvedRespawn.enabled` | Respawn evolved snakes | Bool | Off | reset/world |
| `evolvedRespawn.delaySeconds` | Respawn delay | 0..3600 | 0 | reset/world |
| `evolvedRespawn.delayMode` | Delay variation | Fixed / Uniform | Fixed | reset/world |
| `evolvedRespawn.maxLives` | Lives per individual | 0..10000; 0 unlimited | 0 | reset/world |
| `evolvedRespawn.recurrentState` | Memory across lives | Reset / Preserve | Reset | reset/world |
| `evolvedRespawn.rngMode` | Respawn randomness | Per-slot / World | Per-slot | reset/world |
| `lifeScoring.mode` | Point assessment | Reset / Cumulative / Best Life | Reset | reset/world |
| `lifeScoring.cumulativeBasis` | Cumulative score | Net / Gross earned | Net | reset/world |
| `lifeScoring.cumulativeWallet` | Cumulative boost wallet | Life-local / Carried | Life-local | reset/world |
| `reward.lifeAccounting` | Fitness across lives | Generation totals / Follow score / Legacy terminal | Generation totals | reset/world |
| `reward.durationScaling` | Duration reward scale | Absolute / 240-second normalized | Absolute | reset/world |
| `reward.lifePenalty` | Life penalty | 0..10000 | 0 | reset/world |
| `reward.lifePenaltyBasis` | Penalize | Deaths / Successful respawns | Deaths | reset/world |
| `evolvedRespawn.corpseMode` | Replaceable death food | Normal / None / Subtract replacement mass | Normal | reset/world |
| `world.pelletOverflow` | Full pellet storage | Fail / Discard new excess | Fail | reset/world |

### Sensors and baseline

| Path | UI label | Range/options | Fresh default after qualification | Apply/compatibility |
|---|---|---|---|---|
| `sense.layoutVersion` | Sensor version | v3 / v4 | v4 | reset; structural |
| `sense.profile` | V4 information | Basic / Motion | Motion | reset; structural |
| `sense.bubbleBins` | Angular resolution | 8..32 | 16 | reset; structural |
| existing range/detail settings | Sensor numeric environment | existing bounds | existing selected values | recorded numeric environment |
| `baselineBots.algorithm` | Baseline algorithm | v1 / v2 | v2 | reset; teacher identity |
| `baselineBots.behaviorMode` | Baseline variation | Deterministic / Stochastic | Stochastic | reset; teacher identity |
| `baselineBots.controlDt` | V2 decision interval | 0.008..0.06 | 1/60 | reset; deployment cadence |

Inactive/incompatible controls explain constraints rather than silently coercing. Recorded/imported settings are preserved independently of fresh defaults.

### Distillation and diagnostics

| Path | UI label | Type/range | Initial default | Durability |
|---|---|---|---|---|
| `distillation.enabled` | Teacher capture | Bool | Off | live/job |
| `distillation.seed` | Training seed | UInt32 | 1 | job |
| `distillation.device` | Training device | CPU / available CUDA | CUDA0 when admitted | job |
| `distillation.cpuThreads` | Trainer CPU threads | 1..available | min(2, available) | job |
| `distillation.deviceMemoryMiB` | Device budget | 512..available | 8192 if admitted | job |
| `distillation.learningRate` | Learning rate | 1e-6..1e-2 | 3e-4 | job |
| `distillation.batchSequences` | Sequence batch | 1..256 | 32 | job |
| `distillation.sequenceLength` | BPTT window | 16..256 | 64 | job |
| `distillation.burnIn` | Recurrent burn-in | 0..128 | 32 | job |
| `distillation.ramMiB` | Replay RAM | 64..2048 | 256 initially | job |
| `distillation.spoolGiB` | Dataset disk | 1..64 | 4 initially | job |
| `distillation.shadowCount` | Shadow candidates | 0..4 | 0 | live/job |
| `diagnostics.inspectorEnabled` | Decision inspector | Bool | Off | presentation/live |
| `diagnostics.followIndividual` | Follow population individual | Bool | On | presentation |

Graph/sensor/teacher/cadence/life rule remain immutable job identities. Start/stop/activate/save/import/export/finish remain explicit commands rather than hidden setting mutation.

## 19. Separate future stages

Founders remain a separate Reset/New Run feature using compatible saved brains and ordinary mutation/evolution RNG. No foreign mid-generation insertion is introduced by this plan.

DAgger remains a future student-controlled/advisory-teacher mode using the same committed sample contract without a second simulator.

Exact mid-round recovery remains a separate durability project. Trainer checkpoints and diagnostic counters never imply exact world recovery.

## 20. Emergent interaction review

Continue to observe rather than automatically “correct” the intended evolutionary consequences of the feature set:

- suicide/corpse farming;
- cheap cumulative death;
- gross-score aggressive boost;
- carried-bank newborn boost;
- Best-Life repeated-attempt lottery;
- spending after a recorded best-life peak;
- survival versus aggressive reward tradeoffs;
- cross-life recurrent memory;
- randomized respawn reorientation;
- world-RNG coupling from world-owned effects;
- physical-ID tie changes after replacement;
- stochastic-teacher label variation;
- v3 population-relative moving targets versus v4 local economics;
- long-duration absolute reward scaling;
- wider-network mutation/species effects;
- intentionally limited sensor profiles;
- pellet-capacity ecology;
- spawn congestion;
- structural compatibility versus numeric-environment transfer.

Preserve Rust authority, deterministic transactions, explicit RNG, compatibility and bounded retained capacity. Do not call an evolved exploitation strategy a software bug merely because it is undesirable unless it violates an explicit contract.

## Handoff and branch integration

This file is the updated canonical plan to be handed to the implementation agent together with the infrastructure-simplification prompt.

When integrating it:

1. Treat this file as an explicit owner revision to the existing plan, not as another proposal to audit indefinitely.
2. Replace the canonical plan on the active feature branch with this revision while preserving any genuinely newer product decision made after the source revision cited at the top.
3. If a related experiment branch is retained and still carries its own copy of the plan, update that copy from the same canonical revision rather than allowing divergent process rules.
4. If an experiment branch is deleted, do not copy the plan there merely for archival symmetry.
5. Do not reintroduce the removed implementation diary, historical evidence ledger or per-checkpoint test-count narrative when disseminating the plan.
6. If `AGENTS.md` conflicts with Section 0 by requiring the superseded evidence/CI/test-taxonomy behavior, simplify `AGENTS.md` in the same change so future agents receive one coherent set of instructions.
7. Do not rewrite Git history as part of plan dissemination.

The desired end state is one active behavioral plan, one lean ordinary CI path per project, explicit on-demand qualification workflows, temporary experiment branches, Git-based source provenance, and at most one small shared cross-language contracts layer.
