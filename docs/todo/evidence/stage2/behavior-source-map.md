# Stage 2 TypeScript-reference behavior map

Evidence class: current-source trace plus named executable fixtures.

This map identifies what the Rust migration must study and where the comparison
fixture comes from. “Preserve” means preserve the user-visible or authoritative
meaning, not copy the TypeScript data layout. “Correct” means the current result
is evidence of a defect and must not become a Rust golden master.

| Behavior | Current source path | Existing or Stage 2 fixture | Migration treatment |
|---|---|---|---|
| Configuration defaults and reset | `src/config.ts`, `src/protocol/settings.ts`, `server/config.ts`, `server/settingsSnapshot.ts` | `src/config.test.ts`, `src/protocol/settings.test.ts`, `server/recoveryPhase6.controls.test.ts`, `scripts/stage2/behavior-baseline.ts` | Preserve normalized meanings and atomic boundary application. Do not preserve ignored/invalid input. |
| Graph definition, ports, offsets and parameter count | `src/mlp.ts::buildArch/enrichArchInfo`, `src/brains/graph/compiler.ts`, `src/brains/stackBuilder.ts` | graph unit/integration tests, `scripts/stage2/graph-baseline.ts`, behavior artifact | Preserve graph/port/weight layout compatibility. Correct locale-dependent implicit ordering with an explicit legacy migration order. |
| Neural execution and recurrent state | `src/brains/graph/runtime.ts`, `src/brains/ops.ts`, `src/brains/nativeBridge.ts` | graph integration tests and four-step heterogeneous sequence in the behavior artifact | Preserve accepted numeric tolerance and state sequencing. Correct the count-one, per-node native boundary by evaluating the complete differently weighted due population in Rust. |
| RNG streams and continuation | `src/rng.ts`, `src/world.ts::exportRngState/restoreRngState`, baseline-bot RNG state | `src/rng.test.ts`, determinism tests, behavior artifact | Preserve versioned world/evolution/observer/baseline continuations. Correct external join/resurrection paths that consume authoritative evolution/world streams. |
| World construction and spawn | `src/world.ts::constructor/_initPopulation/_spawnAll/_spawnBaselineBots/_initPellets` and `src/snake.ts::_initBody` | deterministic construction digest; `SPAWN-001` correction fixture | Preserve settings, identity and construction phases. Correct full-body overlap and reject impossible collision-safe population requests clearly. |
| Sensor layout and score-delivery boundary | `src/protocol/sensors.ts`, `src/sensors.ts::buildSensors`, `src/snake.ts::computeSensors/sampleSensors` | sensor tests, first observation vector in behavior artifact | Preserve v3 ordering and delivered `points_delta_norm` semantics. Correct body queries and cap visibility; do not preserve false-clear output. |
| Fixed-step scheduling | `src/sim/SimCore.ts::update`, `server/simServer.ts` tick pump | SimCore/scheduler tests and runtime P0–P4 artifacts | Preserve fixed delta and completed-step semantics. Correct overload starvation, stale catch-up control and silent slow-motion reporting. |
| Control observation and action selection | `src/world.ts::step/_buildControlBatch/_publishControllerSensors`, `server/controllerRegistry.ts` | world-ordering tests, controller tests, Stage 1 browser/server integration | Preserve one observation boundary and newest accepted action before each eligible step. Correct inconsistent observation boundaries and any mixed player/brain control. |
| Browser-player action production | `src/main.ts::buildLatestPlayerAction`, `src/net/playerActionPump.ts` | `src/main.test.ts`, `server/stage1.browserControl.integration.test.ts` | Preserve independent latest-value sending, immediate bounded state-change sends and periodic resend. Measure 30/60 Hz; do not impose this production rule on observation-driven RL. |
| RL Protocol 2 | `server/protocol.ts`, `server/wsHub.ts`, `server/controllerRegistry.ts`, external API documentation | protocol, integration, system and acceptance tests | Preserve the first-cutover JSON contract and observation-driven action production. |
| Movement, boost, growth and food | `src/snake.ts::prepareForStep/applyExternalControl/applyBrainOutput/advance`, pellet grid in `src/world.ts` | snake/world tests; detailed extraction immediately before Stage 5 | Preserve formulas and substep meaning unless a separately reviewed gameplay correction is required. |
| Collision broad phase | `src/spatialHash.ts`, `src/world.ts::_rebuildCollisionGrid` | spatial-hash tests, 200,001-entry regression, P4 artifact | Correct truncation, midpoint-only misses, ignored neighbor range and capacity opacity. Rust storage must be complete or reject the configuration. |
| Collision resolution and credit | `src/world.ts::_resolveCollisionsGrid`, `src/snake.ts::die` | `src/stage2.killCredit.characterization.test.ts`, `COLL-002` correction fixture | Preserve unambiguous body-owner credit. Correct array/grid-order bias; selected simultaneous head-head result is both dead with neither credited unless later evidence proves an intended different rule. |
| Generation/evolution | `src/world.ts::_endGeneration`, `src/mlp.ts::crossover/mutate`, fitness methods | world tests, determinism generation fixture, behavior artifact | Preserve fitness and operator semantics subject to collision/sensor corrections. Preserve every compact eight-field generation summary rather than the current 100-entry in-memory trim. |
| Hall of Fame and resurrection | `src/hallOfFame.ts`, `src/world.ts::_endGeneration/resurrect`, server persistence/API | world/persistence/control tests; detailed extraction before Stage 6 | Preserve run-scoped meaning and compatible resurrection. Replace repeated decimal genomes and global/browser persistence with selected run-scoped indexed storage. |
| Frame v1 | `src/serializer.ts`, `src/protocol/frame.ts`, `src/render.ts`, God Mode parser in `src/main.ts` | serializer/render tests and frame hashes in behavior artifact | Preserve v1 as first cutover target and pack it in Rust. Correct unsafe Float32 ID aliasing by rejecting/routing unsafe identities until a measured later protocol exists. |
| Welcome metadata | `server/simServer.ts::refreshWelcomeState` | source finding and later frame-v1 integration test | Correct the extra full-world serialization; use cached latest packed-frame length metadata. |
| Lifecycle/reset/import replacement | `server/simServer.ts`, `server/wsHub.ts`, controller registry | lifecycle, recovery and priority tests | Preserve sockets, invalidate assignments/epochs, send reliable `state-replaced`, and require rejoin. |
| Checkpoint identity | `server/checkpoint.ts`, `server/snapshotTypes.ts`, `server/persistence.ts` | persistence/determinism tests and behavior artifact checkpoint hashes | Preserve exact generation-boundary identity and compatibility evidence. Replace population-sized SQLite rows with managed immutable checkpoint files plus small SQLite metadata. |
| Current export/import | `server/httpApi.ts`, `server/persistence.ts`, `src/main.ts` | source findings and Stage 6/7 A1–A10 tests | Correct: direct one-file download and direct binary upload; no browser population parse/stringify/Blob reconstruction. |

## Fixture timing

The cross-cutting fixture captures configuration, graph layout, RNG state,
initial authoritative digest, one sensor vector, a four-call recurrent brain
sequence, one complete step, frame v1 and generation-boundary checkpoint
identity now. Detailed movement, collision, command, import and resurrection
fixtures remain deliberately adjacent to the stage that ports them; this avoids
turning Stage 2 into an exhaustive test rewrite before the Rust spine starts.

## Explicit correction corpus

The following are correction requirements, not accepted TypeScript outputs:

- broken body sensing;
- collision-grid truncation and incomplete indexing;
- collision array/grid order bias;
- overlapping spawn admission;
- immediate or mixed neural takeover;
- stale control during catch-up;
- unreliable lifecycle traffic;
- sensor-gated browser action sending;
- connection-driven RNG draws;
- Float32 frame-ID aliasing.

The executable correction fixtures are in `src/stage1.correctionFixtures.test.ts`
and the Stage 1 controller/browser/network tests. Current kill-credit behavior is
separately characterized so the selected corrected rule has concrete before/after
examples.
