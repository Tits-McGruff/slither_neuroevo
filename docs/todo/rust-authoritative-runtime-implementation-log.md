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
| 6 | 6A vertical slice exercised; 6B active | The dedicated experimental server owns durable startup/recovery, continuous Rust frames/stats, browser and Protocol 2 routing, generation persistence, managed retention, and direct archive export/import. Remaining secondary commands, compatibility, durability/performance gates, and cutover work continue. |
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

- 2026-09-09 Explicit socket closes use the same deferred source boundary and
  preserve their original hold/grace deadlines. Stale or duplicate closes do
  not modify a newer lease. The thin Node admission adapter keeps receipt
  retries on one sequence while allowing a required receipt past an unadmitted
  action. Rust capacity/timing tests and the real addon handoff cover both paths.

- 2026-09-09 Token reclaim now retains one same-snake assignment through the
  background queue and exact local-send receipt. Output backpressure and failed
  sends preserve the old lease/token; success rotates the token and rejects old
  socket actions. Reclaim and following actions retain native receipt times.
  Focused Rust tests and the real addon/SQLite handoff cover capacity retry,
  stale receipts during ordinary delivery, and the resumed controller stream.

- 2026-09-09 Legacy reconnect matches exactly one live reserved identity within
  the current run and controller kind. Ambiguous names and explicit invalid
  tokens cannot claim another lease. Identity storage is bounded, charged, and
  preserved by step/generation copies. The real addon handoff exercises legacy
  reconnect after token rotation and generation reassignment.

- 2026-09-09 Fresh joins stage one collision-safe external snake with isolated
  RNG/IDs and charged memory. A separate background delivery barrier publishes
  it only after the exact successful assignment receipt; failed sends and output
  backpressure preserve the source. Queue regressions and the real addon handoff
  cover retry, phase isolation, first observations, and subsequent steering.

- 2026-09-10 Experimental startup now composes the real addon and dedicated
  persistence worker, commits generation one before authority transfer, and
  refuses existing databases. Rust supplies bounded startup metadata without
  serializing game arrays. WebSocket frame leases survive queued replacement
  and in-flight sends. Generation routing shares command admission while
  retaining acknowledgements and send results under backpressure; focused
  tests cover single commit, phase isolation, public IDs, and resume ordering.

- 2026-09-10 `npm run server:rust -- --fresh --db-path PATH` now starts the
  native P0 authority through real HTTP/WebSocket routing and serves the built
  browser. Bounded socket tags and newest unsent player input share admission
  with generation and ordinary receipts. Real socket tests cover frames/stats,
  RL assignment/sensors/actions, disconnect/token reclaim, and explicit command
  errors. A local full default round reached generation two in 239.6 seconds
  with continued delivery and a durable current checkpoint. Normalized native
  settings supply welcome metadata; unavailable collision diagnostics are absent.

- 2026-09-10 The persistence worker can select a bounded current descriptor
  without changing source records. Rust streams and validates the complete
  selected descriptor before retaining a durable boundary, then reuses staged
  activation with its original generation, step, population epoch, RNG and seed.
  Rust generation-two and real addon/SQLite reopen tests cover exact retry after
  a descriptor mismatch, background advancement, and no duplicate publication.
  This primitive rejects ambiguous run selection; automatic recovery is not yet
  wired to server startup.

- 2026-09-10 Restart composition admits only existing managed-metadata databases,
  preserving unrelated reference databases. Recovery commits a distinct run,
  bounded provenance, a source-history prefix reference, and the active pointer
  in one FULL transaction while retaining the failed source suffix and files.
  Rust rebinds only the private restored run identity without copying population
  storage. Transaction rollback/retry, colliding future generations, and real
  addon restart from committed branch provenance have focused coverage.

- 2026-09-10 Normal experimental startup validates the current managed boundary,
  scans inherited retained history newest-first, and commits a recovery branch
  only after Rust privately validates the candidate. Exact managed checkpoint
  IDs never substitute another boundary; exhausted recovery serves health-only
  failure. Health and welcome expose bounded recovery provenance. Focused
  persistence, real-addon startup, and real-socket regressions cover corrupt
  metadata/files, inherited branches, exact rejection, and recovery restart;
  the broad local checkpoint passed.

- 2026-09-10 The built client and actual browser WebSocket/action-pump modules
  now run through the real Rust server in integration: periodic and
  change-triggered latest actions apply steering and boost release without any
  sensor or frame callback producing them. Recovery is visibly marked with
  exact provenance in the browser status. A built-browser smoke also joined and
  steered through the local server. Focused native-server, browser-client, CLI,
  TypeScript, lint, and build checks pass.

- 2026-09-11 The experimental health boundary reports allocation-bounded Rust
  full-step timing and process-local simulated/wall progress alongside Node
  responsiveness, RSS, frame bytes, checkpoint-barrier, accepted-action, and
  controller-lifecycle latency, plus Rust-confirmed player/trainer lifecycle
  totals. Fixed histograms retain scalar evidence without
  copying authoritative state or persisting an unbounded per-step series. The
  first live probe exposed and fixed a nonterminal invariant that incorrectly
  rejected legitimate external-controller RNG draws from boost/death effects;
  the repeated workload continued through replacement and token reclaim. A
  270-second loopback diagnostic crossed generation two with 17,183 timed
  steps, 16,173 trainer actions, 23 assignments, 6 ms step p95/p99, a 36.2 ms
  checkpoint barrier, 0.9997 simulated/wall, and no protocol/routing failure;
  this is not the remaining LAN-browser/owner-trainer signoff.

- 2026-09-11 The live Stage 6A diagnostic now runs the production browser
  latest-action pump beside the Protocol 2 trainer and spectator, pauses all
  browser-player inbound frames/sensors, changes turn and releases boost, and
  requires Rust action application plus inbound recovery. An optional strict
  gate requires observable server-side frame replacement on the real LAN link;
  focused native-server coverage passes without making network-buffer timing a
  cross-platform unit-test assumption. A strict three-second loopback pause
  applied 143 new player actions, replaced 60 display frames, and recovered
  inbound delivery in 29 ms; this remains diagnostic rather than LAN signoff.

- 2026-09-11 PyRL-trainer `ec62b05` now uses Protocol 2, omits the unsupported
  bot `viz` request, and preserves/retries Rust reclaim tokens. That trainer
  revision connected to the real Rust server, discovered 83 sensors, received
  a death replacement, and produced hundreds of Rust-applied actions.

- 2026-09-11 The current browser and PyRL trainer `ec62b05` stayed connected
  through a Rust generation transition over the host LAN address. The trainer
  completed repeated death/reassignment cycles while the browser rendered and
  steered; the integrated sample reported 6 ms step p99, a 35 ms checkpoint
  barrier, approximately 1.0 simulated/wall, and 140 MiB peak RSS. Stage 6B
  now has automatic checkpoint classification/backfill, bounded retention
  inventory, and an exact-current Pin checkpoint command without deletion.

- 2026-09-11 Automatic retention now records a resumable SQLite cleanup intent,
  verifies and removes only unpinned managed checkpoint files, then preserves
  their compact history and Hall-of-Fame metadata as pruned records. Startup
  and each durable generation boundary apply the policy; interrupted cleanup,
  missing-file retry, pins, and the old two-class schema are covered.

- 2026-09-11 Each Rust generation publication now writes the selected winner
  as one validated, content-addressed raw or shuffled-Zstandard weight object.
  SQLite links that object atomically with compact Hall-of-Fame metadata and
  preserves older checkpoints whose legacy winner rows have not been migrated.
  One exact export lease also keeps its selected checkpoint alive across later
  generations and cleanup until the direct download releases it.

- 2026-09-11 The metadata worker now publishes one bounded fixed-width export
  inventory for an exact current-checkpoint lease. Rust fully validates that
  checkpoint and every referenced Hall-of-Fame weight object, composes and
  re-reads one self-contained USTAR `.slither-save`, then Node streams the
  ready file directly and removes it with the lease. The Rust-capable browser
  path activates an ordinary download without reading population bytes.

- 2026-09-11 Raw archive uploads now stream to one bounded synced ready file
  without browser/Node parsing or whole-body buffering. Rust then checks the
  complete outer container, manifest, role hashes/counts, history, Hall of Fame,
  and embedded checkpoint and fully restores a private candidate without
  changing the running game or SQLite current pointer; corrupt files and
  validation scratch are removed cleanly.

- 2026-09-11 Rust Hall-of-Fame storage now retains the best 50 unique
  run-scoped winner objects plus pins while keeping older compact metadata and
  reclaiming unreferenced files. Exact saves contain direct checkpoint roles
  instead of a nested checkpoint archive, deduplicate repeated winners, and
  apply the shared adaptive numeric encoding to aggregate winner weights.
  Interrupted binary responses are terminated without appending JSON, and a
  pinned current checkpoint remains reported as both current and pinned.

- 2026-09-12 Direct import now streams the browser-selected `.slither-save`
  unchanged, privately restores it in Rust, commits its checkpoint, complete
  compact history, selected Hall of Fame and active run in one SQLite
  transaction, then swaps the paused running authority. Failed imports leave
  the old game current; success invalidates old controller state while keeping
  sockets open for a fresh join. Duplicate pinned winner entries share one
  packed object without losing either pin. Focused real-server round trips and
  metadata-import tests cover replay, corruption, cleanup and live replacement.
  Stage 7 retains two scaling/durability follow-ups: incremental Hall-of-Fame
  selection instead of a full-history rewrite, and orphan recovery for an
  interrupted Hall-of-Fame file unlink.

- 2026-09-12 An older same-run archive with retained future history now rejects
  exact replacement and can be explicitly resumed under a fresh durable run
  identity. SQLite preserves the original suffix and records source
  run/generation/checkpoint provenance; Rust reuses the admitted population and
  changes only lineage after the commit. Live replacement and process restart
  both restore the branch, and a rejected exact attempt resumes the old
  scheduler without counting the persistence pause as wall-clock debt. The
  minimal PyRL server adapter at `6938663` also handles replacement while it is
  still waiting for its first assignment on the open socket.

- 2026-09-12 The live Rust server now prepares Reset and New Run as private
  generation-one candidates, writes and commits their run-start checkpoint,
  then swaps authority and invalidates old controller assignments on the open
  sockets. Reset retains the seed, New Run uses OS entropy, and SQLite selects
  the replacement lineage in the same transaction as its current pointer.
  The fixed-P0 route rejects changed settings/custom graphs instead of
  silently ignoring them.

- 2026-09-12 Protocol 2 live settings now replace the complete bounded batch at
  one clean Rust boundary, increment the native config identity, rebuild every
  config-derived scheduler/control/physics cache without copying the world or
  population, and update browser/health state only from Rust confirmation.
  Rust independently enforces the live subset, types and ranges, and fresh run
  replacements preserve the active live values. God Mode move now translates
  the selected Rust snake's head, prior swept position and complete body by one
  bounded delta before the next step, using the exact frame-v1 public ID.

- 2026-09-12 God Mode kill now prepares and commits one Rust-owned operator
  death at the ordered command boundary. It reuses the normal corpse formula,
  owning snake RNG stream and entity-ID allocator, reports the exact dropped
  pellet count, and lets the existing baseline timer and external-controller
  replacement paths handle the dead snake on the next fixed step.

- 2026-09-12 Rust-server stats now carry the newest 120 compact persisted
  generation summaries. The isolated SQLite worker resolves inherited branch
  history and decodes only the fixed small records; Node caches that projection
  for browser charts and refreshes it after durable generations and imports.

- 2026-09-13 The Rust server's Hall-of-Fame list now comes from a bounded
  best-first SQLite-worker projection of the selected run-scoped records.
  The browser receives only scalar identity, fitness, score, length and pin
  state; packed neural weights stay in their managed objects for the pending
  Rust resurrection command.

- 2026-09-13 Hall-of-Fame Spawn now sends one compact generation identity. The
  SQLite worker verifies and leases the exact retained weight object, Rust
  decodes it directly, collision-safely places a new snake with dedicated IDs
  and brain state, validates the complete staged authority, and commits it at
  the ordered command boundary before the worker releases the lease.

- 2026-09-13 The existing Visualizer tab now controls one aggregate Rust
  subscription. Rust re-evaluates only one due neural brain, publishes its
  complete visible layers after the matching step commits, and stops all
  activation-capture work when the last browser unsubscribes.

- 2026-09-13 Rust-server startup now removes only recognized unreferenced
  checkpoint/archive scratch files older than 24 hours. Direct upload and
  download enforce a 60-second no-progress deadline without imposing a total
  transfer-duration limit, and all terminal paths retain the existing cleanup.

- 2026-09-13 Reset now sends the complete normalized setting set into private
  Rust construction. Rust applies compatible live and reset-only values before
  deriving configuration identity, writes generation one, and only then swaps
  the running game; population and baseline-count changes remain rejected.

- 2026-09-13 Reset now sends an explicit default-stack or custom graph through
  a bounded private bridge. Rust independently compiles it, checks its sensor
  width, sizes and initializes every genome, persists the graph in checkpoint
  v3, and returns the exact admitted source graph for restart and New Run.

The next dependency is the remaining compatibility work and Stage 6B gates.
