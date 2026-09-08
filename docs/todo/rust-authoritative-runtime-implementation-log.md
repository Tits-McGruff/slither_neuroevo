# Rust-authoritative runtime factual implementation log

This is the compact execution record for
`docs/todo/rust-authoritative-runtime-plan.md`. Tests, fixtures, CI output and
retained benchmark artifacts are the primary evidence. This file records the
meaningful checkpoint, the commit or working checkpoint when useful, the
important result, and the next material gate. It does not restate test logs,
source listings or the plan.

## Logging rules

Future entries should be one concise entry per cohesive feature/checkpoint, not
one entry per helper or micro-slice. Normally record:

- date, stage and commit/checkpoint identity;
- one short description of the capability or invariant that became true;
- the focused regression/integration evidence that matters, or a brief note
  that the broad checkpoint validation/CI passed;
- a benchmark number only when it is itself a stage/performance result;
- the next material unfinished boundary.

Do not copy full-suite test counts, compiler/build metadata, canonical source
byte counts, temporary paths, resolved setup failures, repeated provenance
boilerplate, or routine reviewer bookkeeping into this log. A reviewer finding
belongs here only when it changed the implementation or leaves a material
uncertainty. Existing detailed reports under `docs/todo/evidence/` remain
available as historical records and should be consulted only when their detail
is actually needed.

## Approved plan

- Owner approval received 2026-07-29 for revision `2026-07-29-draft-4`.
- Exact approved-plan commit: `7971ed2ddbda86891c77def31d980aedf96b4236`.
- Plan blob in that commit: `bad8dafd8304fd5f81c0f37eb812fba8f2adb2da`.
- Earlier Draft 1–3 revisions are superseded and were not approved.

## Current stage summary

| Stage | State | Material result / remaining gate |
|---|---|---|
| 1 | Reference repaired | The TypeScript reference has corrected body sensing/collision admission, controller grace/reclaim, catch-up servicing, browser latest-value control, reliable lifecycle priority and explicit correction fixtures. It remains a selected reference path during migration. |
| 2 | Characterization retained | Windows/Oxygen behavior, performance, persistence growth, codec/checkpoint, browser persistence, LAN and owner-data/trainer evidence is retained under `docs/todo/evidence/stage2/`. The measurements establish the old architecture's bottlenecks and the persistence constraints; they are not Rust acceptance results. |
| 3 | Rust foundation established | Rust state/graph/RNG contracts, coarse bridge, managed checkpoint-v3 codec/metadata worker and Rust→SQLite publication handoff exist. Detailed retained artifacts are under `docs/todo/evidence/stage3/`. |
| 4 | Sensing + heterogeneous inference established | Corrected sensor-v3/spatial indexing, whole-population graph inference, runtime SIMD and the joined control boundary are implemented. Target-host performance artifacts are under `docs/todo/evidence/stage4/`; the known single-worker P1 sensing miss remains a later complete-step/parallelization concern rather than a reason to weaken sensing. |
| 5 | Scalar authoritative fixed-step core established | Movement, food, swept collisions, effects, ambient pellets, accounting, baseline lifecycle/control, controller selection, recurrent takeover, complete control/post-control staging, baseline respawn resolution and atomic nonterminal publication are in Rust. The retained coordinator owns complete nonterminal steps; TypeScript reference mapping remains useful porting knowledge. |
| 6 | In progress, late pre-6A | The bounded background runtime carries generation checkpoint publication, persistence acknowledgement, controller reassignment/results and one authority swap. The production addon can exclusively transfer its durable fresh run to this runtime. Server startup, continuous frame/output delivery and real browser/RL wiring remain before the Stage 6A vertical-slice gate. |
| 7–8 | Not yet accepted | Performance/durability acceptance and production cutover remain future gates. |

## Milestone index

### Stage 1

- 2026-07-29 `c489c7e` corrected the active architecture record and retained the
  disputed Git-history evidence.
- 2026-07-29 `ac43cde2f0c4913c0b416f41f60077b33f019caf` repaired body
  sensing and collision-index admission.
- 2026-07-29 `cf0559722d24a9fbc907e42098c22d8aa87a0d5b` repaired
  controller grace/reclaim, scheduler servicing, browser latest-value control
  and reliable lifecycle output.
- 2026-07-29 `729cbd85a8c332bf56b76b5180cfb0d71f7648c3` added named
  correction fixtures for collision bias, spawn admission, RNG contamination
  and frame-ID aliasing.
- 2026-07-29 `2205ff66b5b758ba42cd8af50329da8e8bb4886b` completed the real
  browser event-transmission reference check; `60ce843` later tightened the
  delivered-sensor/token boundary.

### Stage 2

Stage 2 produced the reproducible behavior/runtime baseline, codec and managed
checkpoint prototypes, persistence-growth and browser import/export failure
measurements, current-server control baselines, retention/write-validation
experiments, legacy-format inventory, P6/P7 characterization, graph fixtures,
owner database/save/trainer audit and Oxygen/LAN measurements. The detailed
machine-readable and narrative artifacts are retained under
`docs/todo/evidence/stage2/`; they should not be recopied into later logs.

Key checkpoint commits include `9a2e1fe`, `341c191`, `9669b6b`, `ffb5148`,
`24dca58`, `6205144`, `fbb6200`, `399dbc1`, `e79ebd5`, `2856386`, `32c6af9`
and `c30ea33`.

### Stage 3

- 2026-08-08 `0f4fe9780aee437c5d5e4cf2ba96f10716bd2c0d` established the Rust
  engine foundation.
- 2026-08-10 `1058acf` added the experimental coarse Node/Rust bridge;
  `ac815e7ffaa60d6e82f2a1c0fab799bc3c477c7e` established persistent state and
  reusable calculation contracts.
- 2026-08-11 `a7e345f`, `0ecad54`, `fb0c0df` added managed checkpoint-v3 and the
  metadata worker; `670041f` added the Rust-to-SQLite publication handoff.
- 2026-08-11 `944e486` retained representative Windows/Debian checkpoint
  round-trip evidence.

### Stage 4

- 2026-08-11 `59215c39c7caa7bcad392446dab5822b0d5cf7b4` established scalar
  graph and heterogeneous-population inference; `69997a0` added coarse
  runtime-selected SIMD.
- 2026-08-13 `58a5ae8` / `07b0ae7` completed corrected sensor-v3 and spatial
  indexing with target-host measurements.
- 2026-08-13 `f8452f3` joined corrected sensing and differently weighted
  inference into the control boundary; `9c38b48` tightened its harness and
  allocation behavior.

### Stage 5

The detailed TypeScript behavior map in
`docs/todo/evidence/stage5/typescript-world-step-map.md` is retained because it
is porting knowledge rather than process bookkeeping.

The main Rust fixed-step chain is:

- `2311ef2` physics transactions and collision-safe spawn;
- `53c1462` ambient pellet generation;
- `64f1a89` once-per-step accounting;
- `7fe57c2` baseline lifecycle timing;
- `95b33a4` baseline strategy/control;
- `fd3c721` joined fixed-step prefix;
- `54a61bd` explicit neural-takeover recurrent reset;
- `33207e5` joined controller selection;
- `92700bf` continuation-state ownership;
- `32f4d06` complete internal control commit;
- `36a4578` complete post-control world-step staging;
- `22ddbdd` collision-safe due baseline respawn resolution;
- `6df8399` atomic running-step publication;
- `2f49952` authoritative configuration projection;
- `0a611f0` private nonterminal authoritative coordinator.

`cf80763` on 2026-08-30 checkpointed the accumulated late Stage 5 / early
Stage 6 working state before the subsequent Stage 6 feature commits.

### Stage 6

- 2026-08-30 `ecb167e` published the initial Rust frame-v1 from the experimental
  fresh-run path.
- 2026-08-31 `729f04a` published the first Rust-scheduled post-step frame.
- 2026-08-31 `7659fef` retained the running authority loop across repeated
  service boundaries.
- 2026-08-31 `0fec33c` installed that loop in the opt-in background
  `EngineRuntime`. Independent review found a real Stop/Fault ordering defect;
  the corrected checkpoint retained one terminal output ordering and exact
  authority ownership.
- 2026-08-31 `44b5b9d` resumed one retained generation transition through exact
  persistence acknowledgement, connected-controller reassignment, the final
  authority swap and scheduler/coordinator rebind. `ba9c0ac` was the Linux
  portability follow-up.
- 2026-09-01 CI/runtime hardening followed (`3e1ee9a`, `63420bd`, `9fa62f5`,
  `c60ea71`, `2e133f2`). These are stabilization of the same boundary rather
  than a new migration stage.

- 2026-09-07 `f59b103` Stage 6 background generation queue checkpoint: complete replies
  reserve bounded output before checkpoint publication, acknowledgement,
  assignment preparation or authority retirement. Queue drainage resumes retained
  commands once; stale generation receipts leave ordinary-step observations
  intact. Rust queue/runtime regressions and the real addon-to-SQLite handoff
  exercise these boundaries. Review corrections order in-flight admission
  against terminal faults and release blocked authority on runtime drop. The
  Windows/Linux Node 22/24 CI matrix passed at follow-up `cc13a59`.

- 2026-09-08 The production addon transfers its activated, durable step-zero
  fresh run to one unstarted background runtime. Rejected transfers retain the
  existing authority; successful transfer disables the session's one-shot
  scheduler and duplicate initialization. Native integration covers real SQLite
  durability, autonomous fixed steps, queued rejection/wake delivery and joined
  shutdown. The coarse Node handle exposes the retained generation queue path.

- 2026-09-08 The production background runtime continuously caches committed
  frame-v1 bytes and coalesced basic stats. One admitted Rust buffer and two
  bounded Node send buffers preserve in-flight bytes while newer frames replace
  unsent visuals. Cached welcome metadata requires no additional serialization.
  Regression tests cover queue priority, undersized-copy retry, shared-memory
  rejection and send-buffer reuse through the real addon. Frame selection pins
  the cache before checking lifecycle priority, closing a reviewed copy race.

- 2026-09-08 Ordinary controller observations and death assignments now leave
  the background runtime as complete bounded reliable batches. Capacity is
  reserved before step preparation and before delivery receipts can publish a
  step. Exact retry and stale/duplicate receipt regressions preserve pending
  messages and prevent duplicate publication; the real addon/SQLite handoff
  continues through the first ordinary observation in the successor generation.

- 2026-09-08 The thin ordinary-controller transport adapter maps Rust messages
  to existing Protocol 2 sensors/assignments and retains bounded local-send
  receipts under input backpressure without resending packets. Unit and real
  addon handoff tests cover partial admission, failed sends and exact public IDs.
  The shared command pump must preserve receipt sequence order while blocked.

- 2026-09-09 Incoming steering uses Rust receipt timestamps and applies only at
  an unprepared pre-step boundary. Actions retained during a delivery or
  generation barrier leave input capacity for its completion; receipt processing
  drains deferred actions before the next step. Output admission precedes lease
  mutation, and delayed actions retain their original hold deadline. Focused
  queue and real addon regressions cover deferred application and exact retry.

The next dependency is controller join/disconnect/reclaim integration and the
shared ordered Node command pump, then experimental server startup.
