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
| 6 | Rust runtime complete | Rust owns durable startup/recovery, continuous frames/stats, browser and Protocol 2 routing, generation persistence, managed retention, commands, and direct archive export/import. |
| 7 | Acceptance active | P0/P1/P2 server timing, LAN steering and desktop drawing, real trainer traffic, large archives and bounded persistence slices are retained. The corrected replacement-token path passes a complete loaded P1 player/RSS/queue soak. Crowded P4 desktop follow/overview drawing passes locally; physical laptop/final-source LAN drawing, archive heap and final durability acceptance remain. |
| 8 | Production cutover active | Normal npm and launcher startup selects Rust; the TypeScript game is retained only as `server:reference`. Deployment/service and final acceptance remain. |

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

- 2026-09-13 The Rust server now saves, lists and loads bounded graph presets
  through its isolated SQLite metadata worker and existing browser HTTP API.

- 2026-09-13 Fresh Rust construction now derives evolved and baseline-bot
  population sizes from validated Reset settings instead of fixing them at
  55 and 10; checkpoint/import limits cover the existing UI ranges.

- 2026-09-13 The direct Rust import route now recognizes older browser JSON
  population files without loading them in browser or Node memory. Rust parses
  bounded genomes from disk, preserves compatible settings, ASCII-safe graph
  layouts and the source seed, then commits the population as a new
  generation-one checkpoint because legacy files lack exact continuation state.

- 2026-09-13 Resume-latest now converts the newest compatible TypeScript v2
  per-genome SQLite checkpoint directly in Rust. It verifies each bounded
  weight row and checksum, commits a new generation-one managed checkpoint,
  and leaves the source snapshot rows unchanged.

- 2026-09-13 The same read-only Rust conversion now streams historical combined
  gzip populations through SQLite's incremental-BLOB API one genome at a time
  and streams format-null/zero parent JSON directly from SQLite. Real startup
  tests cover both layouts and prove the source rows remain unchanged.

- 2026-09-14 Legacy SQLite conversion provenance now commits with the new Rust
  run and survives restart. Health and welcome identify its source row and
  format and explicitly label it population-only rather than exact continuation.

- 2026-09-14 Managed startup now verifies every physically retained checkpoint
  and referenced Hall-of-Fame object before deleting unreferenced final managed
  files. Any failed verification disables deletion; unknown files and links are
  untouched.

- 2026-09-14 The browser now distinguishes recovered runs, exact imported
  branches, and population-only legacy conversions in its server status. Normal
  Hall-of-Fame retention ranks in SQLite and changes only the bounded selected
  set instead of rereading and rewriting all lifetime history each generation.

- 2026-09-14 Checkpoint, import, export, and pin now share the transient-disk
  formula, including existing private work files, operation spools, final
  managed bytes, SQLite/WAL allowance, the temp quota, and 1 GiB operating
  reserve. Import scratch and export inventories use recognizable
  operation-scoped names for safe restart cleanup.

- 2026-09-14 Archive import now decodes the bounded Hall-of-Fame numeric entry
  directly from the uploaded archive into its validated raw work file. It no
  longer retains a second complete encoded copy while constructing managed
  content objects.

- 2026-09-14 Rust-server health now reports SQLite, WAL, SHM, page/freelist,
  managed temporary-file, free-disk, quota, and operating-reserve byte counts.
  SQLite counters come from the existing isolated metadata owner and never
  inspect population payloads.

- 2026-09-19 The isolated SQLite worker now reports correlated, monotonic
  progress during bounded file/record work. Its parent rejects pending work
  and terminates the worker after 60 seconds without progress; health faults
  if subsequent storage inspection cannot use that worker.

- 2026-09-19 Rust export now packs Hall-of-Fame weights in bounded blocks and
  uses its full import validator as the sole full post-write archive scan. This
  removes per-float hash/write calls and one redundant complete archive scan
  without changing the save format or removing pre-download validation.

- 2026-09-19 Native export/import tasks now expose their exact operation,
  start/finish state, and monotonic completed file/codec bytes to Node. The
  server watches preparation for 60 seconds without progress and exits on a
  stuck in-process Rust worker; active transfers retain their separate idle
  limit. Imported Hall-of-Fame weights now verify in bounded chunks. Process
  restart and injected-hang acceptance remain Stage 7/8 gates.

- 2026-09-19 Rust-server startup now rejects a mismatched engine contract
  before creating or modifying a database. Health and browser welcome report
  the verified native build identifier instead of a null placeholder.

- 2026-09-19 A copy of the owner's v2 SQLite database now converts through
  Rust startup. The legacy reader accepts its numeric 0/1 boolean setting;
  a legacy-only failure no longer gets masked as a managed-recovery error.

- 2026-09-19 Rust legacy-v2 conversion now streams parent JSON and genome BLOBs
  directly from SQLite, hashing and decoding weights in fixed 64 KiB blocks.
  A multi-block fixture and a fresh copy of the owner's database pass; source
  rows in the copy still hash exactly like the original after conversion.

- 2026-09-20 The target VM's one-worker 310-snake step averaged 32.3 ms, of which
  about 20 ms was sensing. A selectable persistent Rust pool now partitions
  pure sensing while preserving ordered inference and commits. Four workers
  averaged 18.6 ms and six 17.6 ms on the same 30-step fixture; world and
  recurrent-state hashes matched one worker. The heavy case still misses the
  16.7 ms real-time budget, so further measured optimization remains required.

- 2026-09-20 The same pool now evaluates disjoint brain ranges in parallel.
  Collision profiling found 5.4 ms in all-pairs head checks; a conservative
  swept-envelope rejection reduced that to 1.7 ms without changing the world
  or recurrent hashes. The six-worker P1 step averaged 12.9 ms on the target
  VM, but a 25.5 ms p99 outlier and the missing integrated-load test keep the
  Stage 7 performance gate open.

- 2026-09-20 A real Debian-hosted 300-snake Rust server with six workers,
  browser-player and trainer protocol clients, display delivery, reconnect,
  and a generation checkpoint advanced 0.989 simulated seconds per wall second
  over 90 seconds. The native p99 step bucket was at most 16 ms, checkpoint
  barrier 149 ms, peak RSS 224 MiB; one 779 ms maximum step outlier, archive
  concurrency, actual LAN browsers/trainer, and longer soak remain open.

- 2026-09-20 Import disk admission now uses bounded Rust inspection of the
  uploaded save's manifest instead of two fixed 5 GiB allowances. A 13.8 MB
  same-build save round-tripped on the target VM with about 6 GB free and kept
  its exact checkpoint ID; a different-build save was correctly rejected.

- 2026-09-20 A long 300-snake run exposed winner-object cleanup racing a reused
  weight hash at generation 159. Cleanup now runs only while Rust holds the
  committed generation boundary, before the successor starts; the faulted
  database resumed from its last durable checkpoint. Health now exposes
  discarded scheduler time, and the Linux launcher preserves a failed-resume
  database in health-only mode instead of silently starting fresh.

- 2026-09-21 A ten-minute 300-snake VM run crossed ten checkpoints at 0.986
  simulated/wall speed with no discarded scheduler time. An expired prior-run
  pointer then exposed a retention fault; pruning now detaches only obsolete
  pointers atomically with prune intent. The 55-snake large-brain run resumed,
  crossed two more generations, and directly exported/imported its 75 MB save
  with the exact checkpoint ID. Its 90-second speed was 0.972, below the 0.98
  target, so the large-brain performance gate remains open.

- 2026-09-21 Early rejection of clearly different species comparisons and
  faster adaptive checkpoint compression reduced the large-brain checkpoint
  pause to about 0.53 seconds. The Debian 55-snake large-brain run sustained
  0.984 simulated/wall speed for 125 seconds across two generations, with no
  discarded scheduler time, p99 step bucket 12 ms, and peak RSS 486 MB.
  This uses protocol-compatible diagnostic clients, not LAN browsers or the
  owner's trainer.

- 2026-09-21 Startup now scans only physically retained checkpoint files;
  pruned history no longer increases the managed-file scan. Hall-of-Fame
  metadata migration reads 256 records at a time instead of loading all
  generations into Node memory. Unused winner-object cleanup is paged too,
  with indexed object-reference and legacy-checkpoint lookups; older table
  upgrades restore those indexes after rebuilding the table. Restart fixtures
  cover page boundaries and stale files whose checkpoint metadata is pruned.

- 2026-09-21 On the Debian 300-snake workload, five workers sustained 0.993
  simulated/wall speed with a 16 ms p99 step bucket; four workers sustained
  0.995 but had a 24 ms p99 bucket. New configurations therefore default to
  five Rust calculation workers, while explicit flags and existing config
  files continue to override that default.

- 2026-09-21 Production cutover began: `npm run server`, development watch,
  and both launchers now select the Rust-authoritative server; the old game is
  available only as `server:reference`. A recursive static-import test proves
  production startup does not reach the TypeScript world, scheduler, brain
  pool, or inference worker. Resume-latest creates a first run only when its
  database path is absent; existing failed-resume data remains health-only.

- 2026-09-27 Debian production operation now has a foreground Rust launcher
  and an installable per-user systemd unit with bounded restart policy. The
  online backup command snapshots SQLite, copies and hashes exactly its retained
  checkpoint and Hall-of-Fame objects, retries pruning races, and restores only
  to an absent database-plus-managed-directory pair. On the target VM a fresh
  service resumed the same run after `SIGABRT`; the owner's PyRL client then
  discovered 83 v3 inputs and delivered live trainer actions. Unattended user
  service operation still needs an administrator to enable lingering, so the
  manual launcher remains active there meanwhile. Resume-latest now admits a
  checkpoint across application rebuilds only when its versioned state,
  target, profile, settings schema, and math backend remain compatible; SQLite
  must commit a provenance-labelled branch before Rust can activate it. Exact
  checkpoint selection continues to require the producing build identity.

- 2026-09-27 The resumed Debian run also served a real LAN browser: spectator
  frames, visible Play, steering/boost actions, and reconnect worked against
  Rust; the server accepted 471 player actions during the first play session.
  Production now samples Rust's completed scheduler boundaries from Node and
  exits on a five-second ready-state stall so the supervisor can restart from
  a committed checkpoint. Intentional persistence/controller barriers and a
  paused Node event loop do not count as a Rust calculation stall.

- 2026-09-27 The Debian run reached generation 91 with 11 retained checkpoints
  and a roughly 4 MiB SQLite WAL. An online backup verified 67 managed files;
  its disposable restore restarted from the same checkpoint after `SIGABRT`.
  Corrupting only that copy's newest file exposed that older cross-build
  recovery candidates were tried only as exact-build restores. They now get a
  private compatible-build validation before SQLite branches; the same copy
  recovered the next valid checkpoint, reported one lost generation, and kept
  the damaged file. The live run stayed healthy and directly exported a 3.6 MB
  flat-role save. Pausing one calculation thread in the disposable service
  triggered the five-second watchdog; systemd restarted it from the same valid
  checkpoint. The live lineage has since reached generation 100.

- 2026-09-27 The libuv fresh-run and archive preparation/validation task roots
  now catch Rust panics, return bounded errors, and fault the retained engine.
  Release tests exercise panic containment and rejection of subsequent engine
  commands. Operation-specific injected panics, cleanup checks, the
  administrator-enabled unattended user service, and longer durability gates
  remain open.

- 2026-09-27 Parallel sensing and inference worker roots now attach phase and
  partition identity to a panic before the coordinator faults authority. A
  release test panicked one real Rayon inference partition and verified that
  world and recurrent state were not committed. End-to-end release server
  injection and target-VM performance remeasurement remain open.

- 2026-09-27 A release test injected a synchronous production N-API root panic;
  it returned an error, faulted the retained engine, and joined cleanly.

- 2026-09-27 The manually launched Debian recovery branch remained ready at
  generation 231 after about 100 minutes of process uptime, with 64 generation
  checkpoint barriers, retention cleanup, and 223 MiB maximum RSS reported by
  health. That observation supports continued durability but does not close the
  longer soak or performance gate; the current speed setting reported scheduler
  overload and discarded wall-time debt.

- 2026-09-27 The real SQLite FULL-commit worker now has disposable one-shot
  failpoints before commit and after commit/before reply. The pre-commit test
  rolls back every metadata row and retries; the lost-reply test exits the
  worker, restarts on the committed database, and replays the same descriptor
  with one current pointer and one metadata row. Post-reply Rust swap and
  public-success fault phases remain open.

- 2026-09-27 Browser integration tests now drive the Rust archive capability:
  export clicks a direct download link and import sends the selected `File`
  unchanged. A source-level guard checks those negotiated branches and the
  upload function for population reads or JSON/Blob reconstruction while
  permitting the small import-result JSON. Real large-file browser heap and
  responsiveness measurements remain open.

- 2026-09-27 A real HTTP test held a chunked Rust archive import open, verified
  a concurrent import receives 409, then aborted the first request. Its staged
  upload was removed, a fresh request reached archive validation, and the
  active run/checkpoint identity remained intact. The separate focused spooler
  tests cover the 60-second no-progress rule with shortened test deadlines,
  including a source that emits empty chunks without delivering bytes. An
  opt-in real-socket test exercised the full 60-second connected-peer deadline:
  HTTP rejected the upload, removed its staged file, and preserved the current
  run/checkpoint identity.

- 2026-09-27 A deterministic online-backup test pruned a managed file just
  after SQLite copied its first snapshot. Backup retried from a new snapshot,
  restored the retained file and pruned row, and left no partial directory.
  A still-referenced missing file exhausted bounded retries without publishing
  a backup. A live target-VM prune race remains to be exercised.

- 2026-09-28 A disposable Windows Rust server accepted the approved P2
  55-snake, 402,914-parameter graph through Reset. A 15-second Protocol 2
  probe advanced 899 fixed steps at 0.998 simulated/wall speed with no dropped
  wall debt. Direct HTTP export produced a 74,514,432-byte archive. The in-app
  browser later downloaded a 109,900,288-byte generation-31 archive as one
  ordinary file; an independent TAR reader listed its nine USTAR entries.
  Selecting that exact downloaded file in a second disposable server's browser
  Import control restored seed 42, generation 31, and checkpoint
  `52b729e8558c9508ffe6449363d7f9b68c57695e3b38adb33d612358d561fd0e`;
  the UI reported “Imported run ready.” Renderer heap after import was about
  10.5 MiB, but the browser diagnostic channel could not sample during the
  download response. Peak heap, browser-process memory, responsiveness and
  small-versus-large comparisons remain open, as does the target-VM P2 gate.

- 2026-09-28 The live Debian Rust run remained ready after 43,156 seconds
  (about 12 hours), at generation 550 with 383 completed checkpoint barriers,
  20 retained automatic checkpoints, one pinned checkpoint, a 2.1 MB SQLite
  database, and a 4.3 MB WAL. Its automatic retained payloads occupied about
  35 MB under the 4 GiB cap. A hot online backup copied and verified one SQLite
  snapshot plus 176 managed objects (21 checkpoints and 155 Hall-of-Fame weight
  objects across four runs, 45,027,334 managed bytes); restore verified the same
  set. The restored current checkpoint started ready at its exact saved ID.
  Compact Hall-of-Fame rows remain for older generations, while each run had
  at most 50 selected unique genomes and no unreferenced weight objects in
  the snapshot. On a second disposable restore, selecting the older pinned
  generation-165 checkpoint as current exercised compatible-build recovery:
  exact-build resume correctly rejected it, then latest recovery branched from
  that exact pinned ID and started ready. The live run continued to generation
  555. A timed live backup racing prune and full performance/memory gates remain
  open.

- 2026-09-28 A disposable copy of that backup resumed all 20 retained
  automatic checkpoints, generations 250–551, by exact ID. Each server
  exported a direct archive whose manifest matched the checkpoint's saved
  generation and logical root. The older pinned checkpoint exposed two export
  admission defects after compatible-build recovery: the client rejected the
  valid source-run descriptor on the new recovery lineage, and Rust export
  required exact build identity despite compatible restore admitting the
  source. The client now relies on the worker's durable lineage validation;
  Rust export uses compatible build admission while import remains exact.
  A rebuilt Debian addon in a disposable checkout resumed the pinned
  generation-165 checkpoint as a compatible recovery branch and exported its
  4.9 MB archive with the matching checkpoint ID, generation, and logical root.
  Local gates passed: 453 Rust release tests plus one doc test, 594 JavaScript
  tests, 78 native-required tests, TypeScript, ESLint, Vite, rustfmt, and Clippy.
  The full A9 volume/budget fixture remains open.

- 2026-09-28 Managed import publication now reconciles an uncertain commit
  reply with the durable active pointer before releasing the staged old Rust
  world. If the worker cannot answer or the pointer has changed, the server
  faults and stops authority; a confirmed unchanged pointer permits cancel.
  Real-worker one-shot failpoints proved full rollback before the import commit
  and restart/replay after worker exit following FULL commit but before reply.
  Real HTTP/server tests dropped a successful reply and injected errors before
  and after the Rust swap: each left the newly committed pointer durable,
  returned health-only failure, and restarted from that checkpoint. The
  pre-commit comparison now captures SQLite's actual current pointer after
  Rust staging, not the older startup checkpoint ID. A fast real generation
  advanced that pointer, rejected its older exact import with the expected
  branch-required response, and stayed ready. Focused
  server/persistence tests, 599 JavaScript tests, 81 native-required tests,
  TypeScript, and ESLint passed. Process-death and hang-injection coverage
  beyond these phases remains open.

- 2026-09-28 A preloaded hot backup on the live Debian Rust run was triggered
  ten steps before its durable generation boundary. A separate monitor saw
  both generations 750 and 751 while the backup call was active; the consistent
  SQLite snapshot retained generation 750, and all 176 referenced managed files
  verified. Restoring that set into a disposable database resumed the exact
  generation-750 checkpoint and directly re-exported a 6,887,936-byte archive
  with matching ID, generation, and logical root. The deterministic
  file-disappearance retry remains covered by the existing backup test. This
  live prune-window observation does not close the full A9 volume/budget
  fixture.

- 2026-09-28 A disposable Debian server resumed the generation-750 hot-backup
  checkpoint, advanced live Rust steps, and was killed at the OS process boundary.
  A new process selected the same last fully committed checkpoint and served
  its 6,887,936-byte direct archive; the separate live server stayed ready.
  A cross-platform native system test now kills a child during real steps,
  checks SQLite's committed pointer, then restarts and exports that boundary.
  A second test kills the process during chunked import spooling, ages the
  abandoned file beyond its documented 24-hour grace, and proves startup
  cleanup retains the original pointer. Kill injection during checkpoint,
  export, import commit and prune remains open.

- 2026-09-28 A production-path A9 runner now resets a disposable Rust server to
  the approved P0/P2/P3 graph and settings, advances short eight-second rounds,
  and measures managed files plus live SQLite/WAL, compact metadata and retention.
  Windows probes completed ten P0 generations with three files pruned, one P2
  generation, and one P3 generation. P3 exposed a 256 MiB numeric-role ceiling
  below its 461 MiB raw-weight envelope; the production limit is now 512 MiB.
  Its terminal evolution also outlasted the five-second watchdog, so Rust now
  reports completed coordinator work during evolution and checkpoint encoding.
  The P3 generation completed with a 1.22 GB observed transient peak and 817 MB
  final physical storage. The full 480-generation target-VM fixture and every
  retained-anchor restore/export remain open.

- 2026-09-28 The target-VM P0 fixture committed 480 generations after a safe
  runner resume at generation 63. It retained 22 checkpoint files, including
  one owner pin and one prior-run anchor, and pruned 460; all 480 history and
  Hall-of-Fame rows remained. Peak observed managed-plus-SQLite/WAL storage
  was 54.6 MB, falling to 45.1 MB after shutdown. The reference audit found
  exactly 72 physical managed files for 72 live references, and isolated
  production startup plus direct archive export verified all 22 retained
  checkpoints. The prior-run anchor required selecting that run in the
  disposable metadata copy because exact-ID startup scans only the active
  lineage. P2/P3 volume, legacy compaction, and that prior-run selector gap
  remain open.

- 2026-09-28 An explicit offline legacy compaction command now takes and
  validates a complete backup, admits SQLite temporary-copy disk space, runs
  `VACUUM`, and checks the result. A target-VM fixture used 480 measured-size
  old population BLOB rows, then removed the migrated predecessors. Its
  verified pre-compaction database was 1.215 GB; `VACUUM` reduced the source
  database to 2.54 MB in 11.8 seconds with the newest row retained and no
  freelist pages. The operator must stop the server before invoking this
  command. P2/P3 volume and the prior-run exact-ID selector remain open.

- 2026-09-29 A validated backup restore can now select an exact retained
  prior-run current checkpoint only in its new database copy. The completed
  480-generation P0 backup copied all 72 referenced managed files; selecting
  its prior-run checkpoint in the restored copy started the production Rust
  authority and directly exported an archive with the matching checkpoint
  ID and manifest. The original database kept its active pointer. Direct
  exact-ID startup on an inactive lineage remains a separate gap.

- 2026-09-29 Target-VM P2 and P3 accelerated fixtures each committed 480
  generations and retained all 480 compact history and Hall-of-Fame rows.
  P2 retained 21 automatic checkpoints and one pin, pruned 460, and ended
  with 1.50 GB of automatic files and 1.64 GB of total managed-plus-SQLite
  storage. P3 retained 11 automatic checkpoints and one pin, pruned 470,
  and ended with 4.08 GB of automatic files under the 4 GiB cap; its total
  physical storage was 4.55 GB, including the 406 MB pin. Closed-store
  audits found 72/72 P2 and 62/62 P3 managed files matched live references.
  Isolated production startup and direct archive export verified every
  retained anchor (22/22 P2 and 12/12 P3), including each prior-run anchor.
  P3's sampled transient physical peak reached 5.37 GB (about 4.96 GB
  excluding the pin), so the A9 physical disk-budget gate remains open even
  though post-prune automatic retention met its cap.

- 2026-09-29 The P3 peak led to a disk-admission correction: checkpoint
  publication can hold an adaptive numeric codec candidate beside its final
  archive. Prepublication free-space admission now reserves 528 MiB for that
  candidate in addition to the existing 528 MiB final/winner allowance, WAL,
  and operating reserve. A focused regression rejects free space that met
  the former single-file calculation; startup, TypeScript, and lint checks
  passed. This protects low-disk publication but does not itself lower the
  observed A9 physical peak.

- 2026-09-29 Production generation admission now prunes eligible automatic
  checkpoints before Rust writes its codec candidate and final archive. The
  worker measures managed files plus SQLite sidecars, excludes pinned bytes,
  reserves the simultaneous publication and WAL allowance, and refuses a
  transition if protected anchors cannot fit. A real-worker test preserved
  the current, predecessor, and pin while pruning to a small physical cap.
  A stopped copy of the saturated P3 generation-481 store advanced to 482;
  100-ms sampling found a 4,145,633,827-byte peak excluding its pin under
  the 4,294,967,296-byte limit. All 60 final managed files matched live
  references, and isolated production restart and direct export verified
  all 10 retained anchors, including the pin and prior-run anchor. The full
  A9 gate remains open pending the complete configured-budget and backup
  acceptance scope.

- 2026-09-29 The automatic checkpoint budget is now configurable from 1280
  through 65536 MiB at production startup. On the target VM, a stopped copy
  of the saturated P3 store resumed under a selected 3072 MiB limit and
  committed generation 481. At 100-ms sampling, managed files plus SQLite/WAL
  peaked at 2,923,808,419 bytes excluding the unchanged pin, below the
  3,221,225,472-byte limit. The closed copy had 57 referenced and 57 physical
  managed files, with all 481 history and Hall-of-Fame rows. An isolated
  restart and direct export verified all seven surviving anchors, including
  the pin and prior-run anchor. A repeated probe corrected the fixture report
  to use the selected budget and confirmed the same peak and 3072 MiB
  retention cap. The remaining A9 work covers backup races and copied-set
  recovery.

- 2026-09-29 A hot backup copied a running disposable P3 store at generation
  481 while that source later advanced to 484 under the 3072 MiB limit. The
  backup included 56 managed files and a consistent SQLite snapshot. Restoring
  it to a new database succeeded; isolated production startup and direct
  export verified all six retained checkpoints in that copied set. Backup
  validation now also compares its manifest to the SQLite snapshot's exact
  managed-file inventory. A focused test rejects a manifest missing a
  referenced Hall-of-Fame file while accepting an unreferenced extra file.
  The deterministic prune-during-copy and interrupted-backup cases remain
  open.

- 2026-09-29 A target-VM process kill interrupted a P3 backup after it had
  copied 782 MB. Only an unpublished `.partial` directory remained; no final
  backup directory or manifest was created. A new backup from the unchanged
  source completed with 57 managed files, restored into a fresh database, and
  its generation-484 current checkpoint started and directly exported. The
  abandoned partial directory still occupies disk and needs a safe stale-set
  cleanup path. A deterministic live-pruning collision remains untested.

- 2026-09-29 Future backup attempts now use private partial-directory names.
  Starting another backup in the same parent directory removes only such
  partial sets older than 24 hours whose creating local process has exited.
  Completed backups, fresh attempts and live-process attempts are untouched;
  a focused Windows test covers those boundaries. The older interrupted test
  artifact used the previous name and remains outside this new scavenger's
  recognized set.

- 2026-09-29 A deterministic target-VM backup race used the real persistence
  worker on a stopped copy of the saturated P3 store. Immediately after the
  first consistent SQLite backup, the worker pruned three automatic
  checkpoints under a selected 3072 MiB cap. The file copy detected the
  missing objects, discarded its partial set, took a second SQLite snapshot,
  and published a validated backup with 59 managed files. Restoring that set
  to a new database succeeded. Isolated production startup and direct export
  verified all nine retained anchors, including the pin and prior-run anchor.
  This covers the prune-during-copy race; live Rust stepping during a separate
  hot backup was verified above.

- 2026-09-29 Startup now checks the selected checkpoint budget against the
  protected automatic anchors, live SQLite sidecars, and one bounded
  publication allowance before Rust activates running authority. On a copied
  saturated P3 store, 1280 MiB produced a health-only startup fault requiring
  at least 1,993,406,464 bytes; its active checkpoint pointer and 482 metadata
  rows remained in place. The same copy started successfully at 2048 MiB.
  This protects resumed startup; a fresh run's initial checkpoint is still
  committed before this check, so precommit admission for that case remains
  open.

- 2026-09-29 Fresh startup now also checks the actual Rust-published
  generation-one file against the selected budget before SQLite commits the
  new current pointer. An unsafe boundary removes its still-unreferenced file
  and leaves the handoff unacknowledged. Focused tests prove the check precedes
  commit and acknowledgement, reject the too-small budget, preserve an
  admitted file, and cover the production fresh-start path. A full-sized
  rejected fresh P3 process remains to be exercised end to end. A target-VM
  default P0 fresh start succeeded at the minimum 1280 MiB setting.

- 2026-09-29 Reset and New Run now check the proposed managed file plus the
  existing protected anchors before swapping authority. A target-VM P3 Reset
  at 1280 MiB was rejected before commit with a 1,583,519,704-byte minimum.
  The old P0 current pointer and sole metadata row remained. The first probe
  left an unreferenced proposed file, which restart reclaimed; a follow-up
  correction now removes a newly published budget-rejected file immediately
  after discarding Rust's private candidate. A second stopped probe retained
  exactly one current pointer, one metadata row and one managed file without
  restarting for cleanup. A repeatable target-VM probe then held the same
  production process open after the rejected P3 Reset: health remained good,
  the run and checkpoint identities remained unchanged, and completed steps
  advanced from hexadecimal `0140` at the rejection reply to `0146` afterward.
  On shutdown it still had one current pointer, metadata row, and managed file.

- 2026-09-29 A current-build P3 generation-two fixture on the target VM
  produced a 407,724,032-byte direct archive, above the former 50 MiB JSON
  limit. A repeatable probe streamed that file into a fresh production Rust
  server without a Node population buffer. Import selected the exact source
  run, generation, checkpoint ID, and save root; latest-resume after process
  restart selected the same checkpoint. This proves the large HTTP archive
  path, while real browser download/upload memory and responsiveness remain
  open for A1–A3/A10 acceptance.

- 2026-09-29 A target-VM P3 export overlapping a generation exposed a
  checkpoint-budget fault: export's post-write validator briefly extracts a
  checkpoint into its private `.import-validation` directory, which the
  persistence worker had treated as an illegal non-file. The worker now counts
  that directory and its regular child files in physical budget admission,
  while rejecting unknown directories and links. A focused real-worker test
  verifies the exact byte boundary. A repeated 420,241,408-byte export on the
  target VM completed while the same Rust process advanced through subsequent
  generations with healthy status. The latest CI matrix passed. The in-app
  browser initiated both large and small download requests but canceled each
  at zero bytes; its direct archive URL was also blocked by the browser
  surface. Browser download and memory acceptance remain unproven.

- 2026-09-29 The evolved P0 production server sustained a measured ten minutes
  at 1x on the target VM with five Rust workers and the configured 60-second
  generation: 0.9978 simulated/wall speed, zero dropped scheduler time, and
  eight complete measured transition intervals between 59.95 and 60.22 seconds.
  Health p95 was 2.16 ms, step p99 12 ms, event-loop delay p95 10.72 ms,
  and peak RSS 206 MB. This clears the measured P0 ratio/debt/round/step/health
  portions; browser action latency, display rate, checkpoint-barrier timing,
  real-client load, and the P1/P2 ten-minute gates remain open.

- 2026-09-29 A stopped evolved P0 generation-three checkpoint was copied into
  two isolated stores on the target VM. Sequential continuation with one and
  five Rust calculation workers committed the same generation-four checkpoint
  ID with zero dropped scheduler time in both runs. This is an exact
  next-population check for one P0 boundary; broader worker-count and
  discrete-outcome comparison remains open.

- 2026-09-29 The evolved P1 300-snake production server sustained a measured
  ten minutes at 1x with five Rust workers: 0.9927 simulated/wall speed, zero
  additional dropped time after warm-up, and eight complete measured
  generation intervals between 60.44 and 60.49 seconds. Health p95 was
  2.17 ms, peak RSS 244 MB, and ten checkpoint barriers had a 250 ms
  conservative p95 bound and 244 ms observed maximum. Native step p99 was in
  the 16–24 ms histogram bucket, which cannot resolve the 16.67 ms target;
  the timing histogram now adds an exact gate boundary for a repeat run.
  Real browser/trainer load, action latency and display rate remain open.

- 2026-09-29 The evolved P2 large-brain production server sustained a measured
  ten minutes at 1x with five Rust workers: 0.9807 simulated/wall speed, zero
  dropped time, eight complete generation intervals between 61.17 and 61.47
  seconds, step p99 at or below 16 ms, event-loop delay p99 11.64 ms, and peak
  RSS 399 MB. Ten complete checkpoint barriers had a conservative 1000 ms p95
  bound and 835 ms observed maximum. The speed margin above the 0.98 target is
  narrow; restart-isolated measurement and real browser/trainer load remain to
  be checked before calling the full P2 gate complete.

- 2026-09-29 A restart-isolated P1 ten-minute run from an evolved checkpoint
  reproduced 0.9927 speed, zero dropped time, and short generation/checkpoint
  intervals, but its five-worker step p99 exceeded the precise 16.667 ms gate
  bucket and remains a failed P1 timing target. Short restart-isolated
  six-worker P1 and P2 runs measured 0.9937 and 0.9869 speed respectively,
  zero dropped time, and step p99 at or below 16.667 and 16 ms. Longer six-worker
  runs and real-client load are required before changing the default.

- 2026-09-29 Four isolated continuations from the same stopped evolved P2
  generation-eleven checkpoint used one, four, five and six Rust workers. All
  four committed the exact same generation-twelve checkpoint, history record,
  Hall-of-Fame record, winner weight hash and fitness. The one-worker run
  discarded 5.31 seconds of scheduler wall debt, so this proves one boundary's
  discrete result but does not make one worker a real-time P2 configuration.

- 2026-09-29 The restart-isolated six-worker P1 run then sustained ten minutes
  at 0.9926 simulated/wall speed with zero dropped time, nine checkpoint
  barriers below 243 ms, and complete generation intervals below 60.73 seconds.
  Step p99 nonetheless landed above 16.667 ms in the 24 ms upper bucket,
  unlike its earlier two-minute result, so the required P1 p99 gate remains
  open. The target VM's separate existing game server on port 5174 used about
  1–2 CPU cores during these measurements; no causal attribution or clean-VM
  pass is claimed.

- 2026-09-29 A restart-isolated six-worker P2 run sustained ten minutes at
  0.9810 simulated/wall speed with zero dropped time, step p99 at or below
  16 ms, nine checkpoint barriers below 798 ms, and complete generation
  intervals between 61.21 and 61.44 seconds. Peak RSS was 550 MB. This is
  only a small improvement over the five-worker P2 run and does not establish
  a better default; real browser/trainer load and the P1 timing issue remain.

- 2026-09-29 After `20a66b7` cached swept head bounds for collision pairs,
  a restart-isolated evolved P1 production run with six workers sustained ten
  minutes at 0.9926 simulated/wall speed with zero dropped time. Step p99 was
  in the at-or-below-16.667-ms bucket, nine checkpoint barriers stayed below
  242 ms, and complete generation intervals were 60.25–60.72 seconds. Event
  loop delay p99 was 11.59 ms, health p95 was 2.25 ms, and peak RSS was 298 MB.
  The preceding ten-minute six-worker P1 run missed the step p99 target; this
  isolated run clears that measurement but does not attribute the entire
  improvement to the collision change or prove the real-client load gate.
  CI for `20a66b7` passed.

- 2026-09-29 A disposable six-worker P1 production server on the target VM
  served the current UI to the Windows in-app browser over the trusted LAN.
  The browser rendered live Rust frames, joined as a player, sent pointer and
  held-button input, and reconnected within the grace period to the same
  lease. Rust-confirmed telemetry recorded four fresh assignments, one
  applied disconnect, one successful reclaim, more than 8,800 applied player
  actions, 16 ms action-to-application p95, and zero dropped scheduler time.
  Closing the browser and sending SIGTERM stopped the disposable server; port
  5180 was closed afterward. This covers one real browser/player lifecycle
  slice, not P0/P2, delayed display/sensors, browser frame budgets, or the
  separate desktop trainer under load.

- 2026-09-29 The owner's actual PyRL trainer connected two actors to a
  disposable evolved P1 server over the LAN, discovered the 83-input v3
  sensor contract, received death replacements, and kept learning across an
  intentional server SIGTERM and latest-checkpoint restart. A browser
  spectator was connected during the restarted run. After 1,398 seconds the
  server reported 0.9917 simulated/wall speed, 158,127 Rust-applied trainer
  actions at 16 ms receipt-to-application p95, 11.98 ms Node event-loop p99,
  and 205 MB peak RSS. P1 still failed its complete loaded timing gate: native
  step p99 was in the 16.667–24 ms bucket and the scheduler reported 39.667 ms
  dropped wall debt. The VM's separate existing game server remained active;
  no clean-host capacity or cause is inferred from this run.

- 2026-09-29 The same server restart revealed that the browser rejoined as a
  spectator but displayed the welcome overlay again. `8be3b59` now remembers
  an explicit Spectate choice across reconnects while retaining the initial
  join prompt. A focused browser-state regression, TypeScript, ESLint, Vite,
  and CI passed. After rebuilding browser assets on the target VM, a real LAN
  browser returned automatically to visible Spectating and fresh Rust frames
  across another disposable server restart. The temporary server was stopped
  afterward; the owner's port-5174 game was untouched.

- 2026-09-30 `b568fb5` and `24c60c7` added allocation-free phase attribution
  for ordinary steps above 16.667 ms. In a disposable evolved P1 continuation
  with six workers and two actors from the owner's actual PyRL trainer, the
  second build measured 9,967 steps: 113 exceeded the boundary, native step
  p99 remained in the 16.667–24 ms bucket, and no scheduler wall debt was
  dropped. Of time in those slow steps, 58% was control selection, 38% world
  update and 4% other service. Neural batch evaluation was 81% of slow control
  time (about 47% of slow-step time); spatial-index rebuilding was 5% of slow
  control time. The target VM's separate port-5174 game remained active, so
  this isolates a likely optimization target but does not establish an unloaded
  capacity limit or prove AVX would help. Focused Rust control/phase tests,
  Clippy, TypeScript, ESLint and the native integration overlay passed. The
  disposable trainer/server stopped afterward; port 5180 was closed.

- 2026-09-30 `534821e` separated sensing from graph inference inside the
  production neural-batch timing. A disposable six-worker evolved P1 server
  with two real PyRL actors then measured 5,518 further steps, including 44
  above 16.667 ms, with zero dropped scheduler debt. Sensing accounted for
  84% of neural-batch time in those slow steps; graph inference accounted for
  16%, or about 7% of their complete step time. This makes wider SIMD math an
  unlikely standalone P1 timing fix and points the next profiling/optimization
  slice at sensing and worker waits. The interval is too short and spans a
  changing population, so it does not establish the final p99 gate. Focused
  Rust tests, Clippy, TypeScript, ESLint and the native integration overlay
  passed. The disposable trainer/server stopped; port 5180 closed while the
  owner's port-5174 game remained active.

- 2026-09-30 `6312d82` and `32ac41f` added test-hook-only sensor-phase timing
  to the offline P1 fixture. On the target Ryzen VM, a 310-snake proof pass
  spent about 7.8 ms collecting pellet candidates and 8.2 ms evaluating them;
  together they were about 80% of sensing time. The 30-pass single-worker
  sensing mean was 21.2 ms. This synthetic fixture identifies both pellet
  stages as candidates for focused work; it does not prove loaded production
  timing. An exact-output angle-remainder shortcut was tried, but paired
  pinned-core runs showed no reliable whole-pass improvement, so `305a9e7`
  reverted it. The next optimization should be measured against the pellet
  query or accumulation cost and then the complete loaded step.

- 2026-09-30 The real Rust fresh-run and generation-handoff fixtures now inject
  failures after the SQLite worker replies but before Rust acknowledges the
  committed descriptor. Each database has its committed pointer while Rust
  remains at the old pending boundary; retry replays the exact commit and
  completes acknowledgement and one authority activation or successor swap.
  The isolated production/test-hook addon suite passed. Process-death
  injection at this point and the post-swap/pre-success boundary remain
  separate acceptance work.

- 2026-09-30 Reset and New Run now check the durable current pointer if a
  replacement commit returns an error after staging Rust authority. An
  unchanged pointer permits cancellation; a changed or unreadable outcome
  faults the interface for restart instead of restoring the old Rust run or
  reporting New Run as rejected. A real socket/SQLite test loses the commit
  reply after SQLite activates a New Run and confirms latest-resume loads that
  committed generation-one run.

- 2026-09-30 A separate-process New Run test now kills the actual server
  immediately after SQLite acknowledges the replacement commit, before the
  awaiting Rust/Node call can swap or announce the new authority. The committed
  active-run pointer changes, and a fresh process resumes that generation-one
  run. The Windows process-death file passed all three cases; Linux CI remains
  the cross-platform confirmation.

- 2026-09-30 Food sensing now classifies pellet direction against per-sample
  angular boundaries, with the original angle calculation near boundaries or
  for unusually large restored headings. A 3.5-million-plus direction/bin
  comparison, all 455 release Rust tests, Clippy and the native integration
  overlay passed. On Oxygen, paired pinned-core synthetic P1 sensing means
  changed from 20.12 to 18.03 ms and from 19.50 to 19.13 ms; a final-source
  pair measured 18.20 versus 17.08 ms. Pellet accumulation fell by about 2 ms
  in the first two pairs and 2.5 ms in the final pair. P1, P2 and dense-pellet
  observation hashes were unchanged. This is a sensing-only result; the loaded
  P1 fixed-step p99 gate remains open.

- 2026-09-30 Separate-process recovery now kills Reset, New Run and archive
  import after the SQLite reply/before Rust swap and after the matching Rust
  swap/before public success. All six cases reach their recorded real boundary,
  send no replacement success, and resume the exact committed run/checkpoint
  in a fresh process. Import also leaves its completed upload spool after the
  kill and removes recognized scratch after the documented grace period on
  restart. The Windows process-death file passed all eight cases; TypeScript
  and focused ESLint passed. Generation-transition, export and pruning death
  coverage remain separate work.

- 2026-09-30 Ordinary generation handoff now has separate-process kills after
  the real SQLite reply/before Rust acknowledgement and after Rust's final
  successor swap/before a successor frame or stats reaches the viewer. Both
  resume the exact evolved checkpoint and advance beyond its saved step;
  generation-one history and Hall-of-Fame rows remain unchanged and unique.
  The Windows process-death file passed all ten cases. TypeScript and focused
  ESLint passed; cross-platform CI supplies the Linux confirmation. Export
  and pruning process-death checks remain open.

- 2026-09-30 Export recovery now kills a separate real server after Rust
  publishes its ready archive and during the first binary download write.
  Both leave the selected checkpoint unchanged, retain the interrupted ready
  file and export inventory, remove stale scratch on restart, and permit a
  complete fresh download with normal cleanup. Both focused cases passed on
  Windows after resuming; the unchanged combined process-death file previously
  passed all twelve cases, with TypeScript and focused ESLint passing. This
  does not cover death inside the native encoder or retention pruning.

- 2026-09-30 Pruning recovery now kills a separate real server in its
  persistence worker after committed deletion intent and immediately after
  the actual managed-file unlink, before final classification. Each restart
  completes the pending deletion, resumes the exact current checkpoint and
  preserves the pinned checkpoint, every retained file, compact generation
  history and Hall-of-Fame rows. The combined Windows process-death and
  server-lifecycle files passed all fifteen cases; TypeScript and focused
  ESLint passed. Cross-platform CI remains the Linux confirmation; native
  encoder death and supervised service restart remain separate acceptance work.

- 2026-09-30 A fresh isolated Oxygen build of `7cf4ea3`, using this chat's
  own evolved P1 fixture and two real PyRL actors, measured 600.03 seconds,
  35,746 steps and 0.9929 simulated/wall speed with zero dropped debt or
  sampled overload. Native step p99 was 16 ms; nine complete sampled generation
  intervals were 60.08–60.54 seconds. The window applied 68,189 trainer actions;
  process-lifetime trainer p95 was 0.25 ms, checkpoint-barrier maximum 251.61 ms,
  event-loop p99 11.01 ms and peak RSS 211.35 MB. The LAN health sampler had
  18.61 ms p95 and no failed observations. Raw initial/final health, workload
  identity and transition samples are retained in
  `evidence/stage7/oxygen-ryzen2700/p1-real-trainer-7cf4ea3-20260930.json`.
  This clears the measured P1 server/trainer timing slice; browser/player
  budgets, loopback-health timing and longer RSS/soak acceptance remain separate.
  The sampler passed TypeScript and ESLint. The trainer and disposable server
  stopped, ports 5180/5174 were closed, and their checkout/database/build and
  trainer checkpoint scratch were removed. No other chat's results were used.

- 2026-09-30 Export process-death coverage now includes a kill inside the real
  native encoder. A large-brain checkpoint gives the observer a nonempty
  `.partial` archive while native progress is started and unfinished; no
  `.ready` archive exists. Restart keeps the exact selected checkpoint,
  cleans stale recognized scratch, and produces a complete fresh archive
  larger than the interrupted file. All three focused export-death cases
  passed on Windows, with TypeScript and focused ESLint passing. Linux CI
  remains the cross-platform confirmation; supervised service restart remains
  separate acceptance work.

- 2026-09-30 An isolated Oxygen `d0c70a6` P2 run with six workers, two real
  PyRL actors and one real LAN browser player measured 600.02 seconds and
  35,292 steps at 0.9803 simulated/wall speed, with zero dropped debt or
  sampled overload. Complete generation intervals were 60.99–61.71 seconds,
  native step p99 16 ms, checkpoint-barrier maximum 903.93 ms, Node event-loop
  p99 11.07 ms and peak RSS 371.64 MB. The window applied 66,913 trainer and
  27,241 player actions; server receipt-to-application p95 was 0.25/8 ms.
  A wire trace under suppressed browser sensors/frames shows held boost and
  release on the same snake, with a boost-off frame 51.13 ms after the release
  send; this single trace does not prove a latency percentile. Delivered
  network-frame p95 was 36.4 ms, but animation callbacks stayed near 1 Hz even
  with the in-app tab presented, so the browser rendering gate remains open.
  Raw reports and a screenshot are retained under
  `evidence/stage7/oxygen-ryzen2700/p2-*-d0c70a6-20260930.*`. The source build,
  both fixture copies and trainer scratch were removed after stopping all
  disposable processes and verifying ports 5180/5174 closed. Sampler/observer
  lint and sampler TypeScript passed. The `d0c70a6` Windows/Linux CI matrix
  passed. The measured P2 ratio is close to the minimum and establishes little
  additional capacity; rendering, loopback-health and longer RSS gates remain
  separate work.

- 2026-09-30 Release tests now panic inside the real fresh-run, export, import
  validation and import preparation task computations after nonempty private
  files exist. Unwinding removes unpublished checkpoint/archive files,
  extracted import files and the prepared inventory; retained checkpoint and
  upload bytes stay unchanged, and no replacement candidate becomes available.
  The task root returns a bounded error, faults its retained engine, rejects
  later commands and joins cleanly. The release Rust suite and Clippy passed.
  These injection points compile only into Rust unit tests. Full Node/server
  panic injection and supervised service recovery remain separate acceptance.

- 2026-09-30 A real release-addon Rayon inference-partition panic now passes
  through the production HTTP/WebSocket router in an isolated startup fixture.
  Health stays reachable with HTTP 503 and the partition fault; settings receive
  an unapplied result. Completed-step counters stay at zero, emitted frame/stats
  state never advances past the valid startup boundary, and the current SQLite
  pointer, managed-file inventory and selected checkpoint bytes stay unchanged.
  Closing and restarting from that exact checkpoint advances normally. The
  separate source-checked test-hooks addon exposes injection only before start;
  normal production startup rejects it and the production addon has no trigger.
  Both server cases, the native-required overlay, release Rust feature suite,
  Clippy, rustfmt, TypeScript, ESLint and browser build passed on Windows. Linux
  CI remains the cross-platform confirmation. This closes the calculation-panic
  transport slice; supervised service recovery remains separate acceptance.

- 2026-09-30 On Debian, an isolated source-checked release test addon now
  injects a real calculation panic under the checked-in systemd restart,
  backoff and stop policy. The caught fault keeps the service active with
  HTTP 503 and rejects settings; it does not silently exit or auto-restart.
  An explicit service restart changes the process ID, restores the same run,
  generation and checkpoint, preserves independently inspected SQLite rows,
  managed-file inventory and checkpoint hash, and accepts a new real WebSocket
  welcome. The unit stops cleanly with status zero and no restart loop. Raw
  health, supervisor properties, durable state and journal evidence is retained
  in `evidence/stage8/oxygen-ryzen2700/caught-panic-eca5a82-20260930.json`.
  The explicit fixture entry point replaces only startup composition and never
  changes production provenance validation. All three Windows server/process
  panic tests, TypeScript, focused ESLint and service/category contracts passed;
  README now explains owner-directed recovery for caught faults. The disposable
  unit and checkout/build/database were removed and ports 5174/5180/5181 stayed
  closed. Earlier abort/watchdog service evidence remains separate; unattended
  startup still requires administrator-enabled user lingering.

- 2026-09-30 The `a22f04b` production P1 workload with six workers, two real
  PyRL actors and a programmatic frame-receiving player completed 1,800.35
  seconds at 0.9923 simulated/wall with 107,192 steps, zero dropped debt and
  16 ms step p99. After ten minutes of warm-up, the twenty-minute RSS slope
  was 0.927 MiB/minute; final RSS was 224.8 MiB, 35.2 MiB above the warm median,
  clearing the 1 MiB/minute and 64 MiB limits with limited slope headroom.
  Loopback health p95 was 1.73 ms, complete generation intervals 60.16–60.70
  seconds, and checkpoint-barrier maximum 239.9 ms. The window applied 206,981
  trainer actions; the player received 53,319 frames and completed 59 successful
  same-snake reclaims across 60 connections. Sampled temporary managed bytes
  stayed zero, automatic checkpoint bytes stayed below the configured cap,
  and WAL stayed below 2.8 MiB. Raw reports are retained as
  `evidence/stage7/oxygen-ryzen2700/p7-p1-*-a22f04b-20260930.json`.
  The reusable sampler now computes the exact RSS gates; its summary/category
  tests, TypeScript and focused ESLint passed, and `a22f04b` full CI passed.
  All disposable processes and source/build/database/trainer scratch were
  removed with ports 5174/5180/5181 closed. This proves the measured P1 memory,
  loopback-health and programmatic lifecycle slice; browser rendering/heap and
  complete queue/backpressure acceptance remain separate.

- 2026-09-30 Production health now exposes native queue occupancy, immutable
  limits, lifetime peaks, rejection/overflow/wait counters and replaceable-output
  decisions through a separate scalar diagnostic query. Normal frame copying
  and frequent step-health reads do not invoke it. WebSocket failure/replacement
  totals and per-connection reliable queue peaks survive peer removal, including
  callbacks that fail after disconnect; no closed peer is retained for reporting.
  The loaded-workload sampler preserves these measurements with resource samples.
  A real HTTP/WebSocket/native integration test holds server display admission
  above its transport-buffer threshold while actual TCP JSON delivery continues:
  fresh assignment and same-snake reclaim arrive exactly once with a rotated
  token, zero reliable failures, bounded peaks and continued stepping; frames
  resume after pressure is removed. A real native queue saturation/drain test
  verifies rejection and peak retention, and a hub regression covers late failures.
  All 460 release Rust tests, rustfmt, Clippy, 102 required native/MT tests,
  13 reference/hub tests, TypeScript, ESLint and browser build passed. The existing
  full upload-deadline test remains opt-in. Test scratch was removed and Oxygen
  ports 5174/5180/5181 were closed. This is scoped transport/diagnostic evidence;
  sustained queue acceptance and browser rendering/heap gates remain open.

- 2026-10-01 Native output diagnostics now retain count/byte peaks separately
  for reliable messages, discrete events, frames and stats. The loaded sampler
  checks each class against its own immutable capacity and rejects incomplete
  or regressed evidence; the Rust drain regression proves peaks survive after
  occupancy reaches zero. The `c67e395` P1 programmatic player completed 1,830
  seconds, 61 connections and 60 actual successful same-snake reclaim replies
  with rotated tokens and no missing replies. Raw client and final-health
  reports plus scoped provenance are retained as `p7-queue-p1-*-c67e395-*`.
  The full queue/RSS sampler was not started because the former combined peak
  could not establish each class's peak; a corrected sustained run remains open.
  Release Rust, native/MT, queue/RSS summary, TypeScript, lint and browser build
  checks passed. All disposable processes and copied source/build/database/
  trainer state were removed; Oxygen ports 5174/5180/5181 remain closed.

- 2026-10-01 The exact `49f5919` P1 production source completed a 1,800.44-second
  loaded window with two real PyRL actors and a reconnecting programmatic
  player: 0.9919 simulated/wall, zero dropped debt and 16 ms step p99. Separate
  native queue peaks stay within their capacities, with no priority overflow
  or fault-discarded commands. The player received all 60 successful same-snake
  reclaims; sampled file descriptors stay at 31–35 and threads at 19. Sampled
  temporary bytes return to zero, with automatic checkpoint bytes and WAL
  within their configured envelope. RSS slope is 1.124 MiB/minute, failing
  the 1 MiB/minute gate despite a final increase of only 28.1 MiB above the
  warm median. Raw reports and measurement provenance are retained under
  `evidence/stage7/oxygen-ryzen2700/p7-p1-*-49f5919-20261001.*`.
  The sampler now preserves heap/external-memory samples for attribution;
  diagnosis and a corrected RSS run remain open. Production dependency
  checks now follow value imports, dynamic imports and CommonJS loads,
  reject unknown computed loads, and exclude the old TypeScript runtime
  and per-layer bridge. Focused contracts, TypeScript and lint passed;
  `49f5919` full CI passed. All disposable processes and source/build/database/
  trainer scratch were removed after verified report copies, with ports
  5174/5180/5181 closed. Browser rendering/heap, complete LAN input timing
  and final acceptance auditing remain separate.

- 2026-10-01 Allocator attribution on exact `255999c` P1 source released
  43.9 MiB of RSS in 3.45 ms with essentially unchanged live allocation
  counters after one controlled GNU allocator trim. The twenty-minute trace
  used two real PyRL actors, a later programmatic player and diagnostic
  instrumentation; its 24 ms step p99 and incomplete player report are not
  acceptance results. Raw allocation, heap/isolate, resource and workload
  reports plus scope and reproduction sources are retained as
  `evidence/stage7/oxygen-ryzen2700/memory-profile-*-255999c-20261001.*`.
  GNU/Linux production now requests release of unused allocator pages after
  successful completed-generation publication and worker rebinding. Windows
  allocator behavior is unchanged. All 498 release Rust feature tests, three
  allocation tests, one doc test, rustfmt, Clippy, 17 real-server tests,
  TypeScript, ESLint and browser build passed; the existing full upload-deadline
  test remains opt-in. The GNU production addon built and advanced a fresh P1
  fixture through a durable generation transition. `255999c` full CI passed.
  The diagnostic checkout/build/database and trainer scratch were removed
  after verified report copies; game, measurement and inspector ports are
  closed. A normal thirty-minute RSS/timing run on the corrected source
  remains required; allocator retention does not establish all RSS growth.

- 2026-10-01 Exact `0a50949` production source with completed-generation GNU
  allocator release completed a 1,800.12-second P1 server window with two real
  PyRL actors: 0.9921 simulated/wall, 107,150 steps, zero dropped time and
  16 ms step p99. RSS slope was 0.712 MiB/minute, final RSS 178.2 MiB and
  29.4 MiB above the warm median; queue peaks passed their immutable limits.
  Checkpoint-barrier p95/max were 250/257.3 ms, health p95 1.89 ms and threads
  stayed at 19. The reconnecting player stopped at 780.5 seconds after its
  same-snake assertion failed; later diagnostic clients changed the client mix.
  These server measurements are scoped evidence, not complete P7 acceptance.
  Raw reports and executed supervisor/reconnect sources are retained as
  `evidence/stage7/oxygen-ryzen2700/p7-p1-*-0a50949-20261001.*`.
  Near-boundary reconnects were consistent in a focused trace; crossing a
  completed generation returned an explicit invalid-token result. The original
  successful-reclaim mismatch still needs decisive correlation evidence.
  A new client-side steering probe times actual sends through observed Rust
  heading reversals, ranks unknown attempts beyond all finite latency bounds,
  and respects the bot route's one-action-per-tick allowance. P1 LAN player
  steering completed 200 trials at 37.4 ms p95. The bot probe stopped at a
  generation-boundary observation deadline after 188 completed/one unknown
  attempt; its joint gate remains open. The probe now continues after an
  isolated observation deadline with that attempt retained as unknown.
  Five correlation/percentile tests, category completeness, 18 real-server
  tests, TypeScript and full ESLint passed; the existing full upload-deadline
  test remains opt-in. `0a50949` full CI passed. The owned checkout/database/
  addon and trainer scratch were removed after verified copies; game and
  measurement ports are closed. Complete LAN P0/P1/P2, player lifecycle and
  browser rendering/archive-heap acceptance remain open.

- 2026-10-01 The loaded player probe now retains each requested snake/token
  fingerprint and its actual reclaim-result/assignment pair before checking
  identity or token rotation. A rejected token triggers one explicit fresh
  join, recorded separately from same-snake reclaim; packets from a replaced
  socket cannot alter the current client identity. A real Rust socket test
  verifies same-snake reclaim followed by explicit rejection/fresh join after
  a durable generation change. Five correlation tests, category completeness,
  19 real-server tests, TypeScript and full ESLint passed; the existing full
  upload-deadline test remains opt-in. The original long-run mismatch still
  requires an instrumented loaded run; this checkpoint changes measurement
  correlation, not production controller behavior.

- 2026-10-01 Exact `4b7f6b0` loaded P1 reproduced the successful-reclaim
  mismatch after 720.5 seconds: request snake 4365 received matching successful
  result/assignment for successor 4682 with a rotated token. Raw request/reply
  fingerprints, the interrupted server window, scope and reproduction sources
  are retained as `evidence/stage7/oxygen-ryzen2700/p7-p1-*-4b7f6b0-20261001.*`.
  A focused real-server regression reproduces the same failure by rejecting a
  successor assignment at the transport boundary. Rust had retained the
  predecessor token on the fresh snake after failed delivery; replacements now
  invalidate that token on either send outcome while preserving neutral control
  and disconnect grace. Same-live-snake reclaim still rotates only after its
  successful send. Full release Rust, rustfmt, Clippy, the rebuilt-addon
  real-server suite, reconnect/dependency contracts, TypeScript and full ESLint
  passed. External-client instructions document explicit fresh-join recovery.
  `4b7f6b0` full CI passed. All owned processes and remote/local scratch were
  removed after verified report copies; the owner game remains stopped. This
  interrupted run does not close the thirty-minute loaded soak; that run and
  complete LAN/browser/final acceptance remain required on corrected source.

- 2026-10-01 Exact corrected `2f1e4b5` production source passed the complete
  loaded P1 server/player soak: 1,800.32 seconds, 107,195 steps, 0.9924
  simulated/wall, zero dropped time and 16 ms step p99. Final RSS was 178.8 MiB,
  29.1 MiB above the warm median, with a 0.849 MiB/minute slope; all queue peaks
  stayed within their limits. Health p95 was 1.75 ms, checkpoint-barrier
  p95/max 250/245.2 ms and the longest complete generation interval 60.70 seconds.
  The player completed 1,830 seconds and 61 connections, including 58 same-snake
  reclaims and two explicit invalid-token/fresh-join recoveries. Separate
  Windows-to-Oxygen LAN probes with two real trainer actors recorded 200
  steering attempts per client route: player/bot p95 upper bounds were
  35.3/36.3 ms for P0, 38.1/38.4 ms for P1 and 38.4/42.5 ms for P2. Unknown
  responses remained in the percentile calculation. Raw reports and executed
  runners are retained as `evidence/stage7/oxygen-ryzen2700/*-2f1e4b5-20261001.*`.
  The sampler now also enforces its already-reported 100 ms health-p95 limit in
  the pass flag; TypeScript, focused lint and queue/RSS contracts passed.
  `2f1e4b5` full CI passed. Reports matched their copied hashes; all owned
  processes, remote checkout/addon/build/databases and local trainer scratch
  were removed, and the owner game remains stopped. Browser rendering,
  large-archive heap/usability, unattended service startup and final acceptance
  auditing remain; these reports do not establish those separate gates.

- 2026-10-01 Exact `14bce5f` production source passed separate 60-second
  Windows desktop browser drawing samples over LAN, each with two actual PyRL
  actors and one browser player. P0/P1/P2 drawing intervals at most 40 ms were
  100.00/99.97/99.88%, with 16.8/16.9/16.9 ms p95; incoming-frame intervals
  met that limit at 98.96/99.27/96.26%. P2 retains a 1.55-second interval stall
  while opening graph/settings UI. The observer now times the actual drawing
  callback and finishes bounded captures independently of diagnostic RPCs.
  A background P0 capture exposed in-app compositor throttling despite visible
  page flags; showing the host and `Page.bringToFront` restored normal cadence
  before the separate full foreground sample. Reports, screenshots, source/load
  health brackets and executed runners are retained as `browser-*-14bce5f-20261001.*`.
  Large P2 and small P0 Export clicks returned no completed browser download;
  archive heap/usability is unproven. Laptop/P4 and final durability/service
  acceptance remain. Focused observer lint and independent report/source/load
  checks passed. All owned processes, QA tabs, the remote checkout/addon/data
  and Windows trainer scratch were removed after compact evidence was retained;
  owner ports remain closed.

- 2026-10-01 Exact `a700d86` production startup passed full-size P3 legacy
  SQLite conversion admission: 300 snakes, 402,914 parameters per genome and
  147 sensors. A 1280 MiB budget returned HTTP/WebSocket 503 before any managed
  file or current pointer remained; the admission requirement was 2,065,430,320
  bytes. Retrying at 1986 MiB committed one generation-one checkpoint and normal
  restart selected that same run/checkpoint. All 300 legacy genome rows and
  parent metadata remained hash-identical. Reports and the reproducible runner
  are retained as `p3-startup-budget-*-a700d86-20261001.json` and
  `scripts/stage7/legacy-p3-startup-budget.ts`. RSS is cumulative across attempts
  in one process, not an isolated memory gate. This covers supported legacy
  conversion, not a P3 `--fresh` configuration or evolved real-time performance.
  TypeScript, focused lint and the conversion-provenance contract passed;
  `a700d86` full CI passed. The owned checkout, addon and copied databases were
  removed after verified evidence retention. Laptop/P4 rendering, archive
  heap/usability, unattended service startup and final acceptance remain.

- 2026-10-01 README now publishes the measured P0/P1/P2 six-worker workloads,
  corrected P1 soak, LAN steering and desktop drawing results, plus P3 startup
  capacity and the remaining acceptance limits. Figures were checked against
  retained reports; service/update/backup/recovery instructions remain present.
  This documentation checkpoint does not qualify laptop/P4 or browser archive
  memory/usability and does not declare final production acceptance.

- 2026-10-01 Dense-world preparation exposed two production capacity defects:
  the inherited 100,000-point body allowance could not reach P4, and the 25,000
  pellet allowance equalled the supported ambient target, leaving no space for
  an ordinary corpse. New authorities now admit 1,000,000 body points and
  250,000 pellets, charged with their frame storage before activation; exact
  older checkpoints preserve their limits. A regression reproduces the old
  corpse failure and verifies the complete death plus the next ordinary step.
  Long steps also starved frames by reserving an eventual reply throughout
  computation. The preceding committed frame can now be copied during that
  reservation while queued replies and terminal closure still take priority;
  its red/green regression preserves those ordering and immutable-cache rules.
  All 463 release Rust tests, the compile-fail doctest, rustfmt, Clippy,
  TypeScript, ESLint and 33 real-server/frame contracts passed (one opt-in
  upload-deadline test skipped). The bounded production capacity runner counts
  actual adjacent-point segments, with live health/queue brackets, and requires
  further frames beyond 200,000 segments. P4 timing/LAN/browser and final
  acceptance remain separate gates.

- 2026-10-01 The same production native source, committed as `8d38fb9`, then
  completed a bounded Oxygen capacity run through normal population conversion
  and supported dense/high-growth settings. Actual frames reached 203,536 body
  segments across eight living snakes (at least 203,304 collision segments after
  configured head skips), with 24,999 pellets and a 2,128,620-byte frame. Further
  frames stayed above the boundary for 5.16 seconds before orderly close;
  1,558 frames arrived over 265.94 seconds, with no authority fault or reliable
  send failure. Peak RSS was 345 MiB. This clears the narrow >200k capacity
  boundary, not its timing/LAN/browser gates: the stress workload reported
  overload, 0.0976 simulated/wall speed and a 256 ms p99 histogram upper bound.
  The report and exact build/source/runner manifest are retained as
  `p4-capacity-*-56711b23-20261001.json`. Unused Windows scratch, the remote
  build tree, and the stopped remote checkout/addon/databases were removed;
  owner service and ports remain stopped/closed.

- 2026-10-01 Collision cell indexing now groups by cell alone; complete queries
  still deduplicate every entry and sort candidates into canonical segment
  order. A >200k-candidate regression checks complete traversal, ordering and
  reuse after reversing equal-cell entries. The explicit cold dense-step
  profiling mode keeps ordinary deaths enabled and checks controls against
  the source observation boundary. Its three Oxygen pairs reduced index time
  by 43–45%, with identical final world/recurrent hashes and work counts;
  all 310 synthetic snakes die normally, so these are diagnostic profiles.
  A fresh production after/before pair reached identical 202,011-segment peak
  frame counts and 1,547-step boundaries. Mean step time was 255.0 versus
  284.4 ms; observed p99 upper bounds were 422.3 versus 487.7 ms. Candidate
  peak RSS was higher, 361 versus 321 MiB. Both runs stayed healthy and within
  queue limits but reported overload; the earlier 169.9 ms capacity mean was
  not reproduced by the restored-sort run. Paired reports/manifests are retained
  with the owner's subsequent report of Plex/media-preview-generator CPU
  contention on Unraid; the timings do not qualify an unloaded target VM.
  They are `p4-physics-paired-*-875f030-20261001.json` and the corresponding
  `p4-capacity-*-20261001.json` files. Engine regressions, normal-addon bridge,
  frame and real-server contracts, rustfmt and Clippy passed; `875f030` CI
  passed. The stopped remote checkout, addons, builds and copied databases were
  removed. P4 timing/LAN/browser and final acceptance remain open.

- 2026-10-01 A real browser inspection exposed an unsuitable P4 preparation
  shape: 98.7% of the audited points were straight tail extensions outside the
  arena. The earlier runs remain storage/frame-capacity and diagnostic timing
  evidence; they do not qualify crowded in-arena collision or drawing. The
  bounded browser observer now records actual frame counts and view modes.
  LAN preparation audits in-arena collision segments after the head skip and
  uses bounded bodies, a larger arena and normal fast movement. Ordinary deaths
  still prevented its >200k target. An early-exit regression run honestly
  reported failure after 27.65 seconds, while healthy, then closed the server.
  Three geometry regressions, test-category checks, TypeScript and focused
  ESLint passed; `93d6e6d` CI passed. Compact reports/screenshots and a read-only
  Carbon CPU/load/pinning sample are retained. Task-owned remote processes,
  checkout, databases and builds were removed; owner service stays stopped.
  A sustained >200k in-arena fixture and final P4/laptop/archive gates remain.

- 2026-10-01 Supported long-body startup exposed unnecessary all-segment spawn
  comparisons: 300 initial 140-point bodies in a 10,000-radius arena exhausted
  the unchanged 10-million geometry-check budget at slot 32. Complete cached
  body bounds now reject provably distant pairs; nearby bodies retain the exact
  segment predicate, stable ordering and candidate draws. The red/green case
  admits all 42,000 points using 2,004,872 checks. Regressions cover bent bodies,
  threshold tangencies, an unfiltered collision oracle, and exact accepted
  positions/RNG continuation. A real Protocol 2 Reset also publishes all 300
  complete bodies inside the arena through the normal production addon. All
  468 release Rust tests, the compile-fail doctest, rustfmt, Clippy, production
  build, TypeScript, ESLint, Vite and 34 server/addon contracts passed across
  focused runs (the upload-deadline test remains opt-in). `d1d1def` CI passed.
  Test sockets, servers and copied databases were closed/removed. This fixes
  startup admission; sustained P4 and final acceptance remain open.

- 2026-10-01 Long-body preparation now starts the supported 300-snake
  population with 140-point bodies and a bounded 5,000-point maximum. A
  separate loopback mode qualified the fixture locally before LAN exposure.
  Oxygen then delivered 982 audited frames over 224.30 wall seconds and held
  >200,000 in-arena collision segments continuously for 180.22 wall seconds;
  peak was 248,149 qualifying segments, with ordinary deaths continuing. At
  peak, 4,424 of 255,903 points were outside the arena. This is sustained
  capacity, not realtime: only 16.37 simulated seconds advanced, mean step was
  227.0 ms, p99 upper bound 369.8 ms, and peak RSS 407.2 MiB. No authority fault
  or reliable-send failure occurred. Full-minute foreground desktop captures
  covered overview and a separately audited bounded replay in follow view.
  Follow had 16.8 ms p95 render intervals, all below 40 ms; overview failed at
  500.5 ms p95. Neither capture had malformed frames. Small-graph inference was
  only 0.08% of measured slow-step time; world work dominated. Reports,
  screenshots, runner/build identities and the reduced Carbon load sample are
  retained as `p4-crowded-*-83402fa-20261001.json` and the corresponding view
  files. Geometry regressions, TypeScript and focused ESLint passed; `83402fa`
  CI passed. All test servers, task-owned checkouts/builds and copied databases
  were stopped/removed after evidence checksum checks. Overview drawing,
  laptop/archive acceptance and final production gates remain open.

- 2026-10-01 Distant rendering now omits subpixel glow and draws food below one
  CSS pixel in diameter with equal-area marks. Body drawing uses bounded chunks
  and reusable scratch to keep curves within half a CSS pixel; complete input
  geometry and close-up paths remain intact. Regressions cover curve error,
  endpoints, returning tails, sharp turns, food centers/area and near-view detail.
  Full-minute foreground desktop captures of real crowded frames passed:
  overview p95 33.5 ms with 99.3% of intervals below 40 ms; follow p95 16.8 ms
  with every interval below 40 ms. A short repeated-baseline comparison on one
  ordinary frame (43 snakes, 448 points, 3,506 pellets) improved overview p95
  from 66.7 to 16.8 ms. This is browser-specific drawing evidence, not a brains,
  sensors or whole-simulation speed claim. Reports and source identities are
  retained as `renderer-*-ab34b87-20261001.*`; Rust capacity evidence from
  `83402fa` remains applicable. Renderer/serializer tests, TypeScript, focused
  ESLint and Vite passed; preceding `ab34b87` CI passed. Test processes and
  copied databases/scratch were stopped/removed. Physical laptop/final-source
  LAN drawing, archive heap and final production acceptance remain open.

- 2026-10-02 The normal browser Export/Import controls downloaded a 2,521,088-byte
  default save and round-tripped an 89,509,376-byte evolved P2 save as its original
  on-disk File. Independent bounded TAR/Zstandard decoding verified every role's
  counts/hash and the save root; the successful HTTP receipt preserved the exact
  run, generation, completed step, checkpoint and root. A traced browser download
  made one request with matching filename/headers/body. An earlier valid download
  had a generation-seven label and generation-eight body; its cause and a traced
  generation-overlap check remain open. Precise peak/post-collection browser heap
  remains unproven. Compact evidence, a standalone verifier and the restored view
  are retained as `browser-*-df2d81d-20261002.*`. Test processes, copied databases
  and scratch were removed; two verified archives remain in the named ignored
  local fixture directory for those unfinished gates. A focused native socket test
  and ESLint passed after reducing admission-fixture movement, preserving all
  300-body assertions and the existing deadline.

- 2026-10-02 A bounded real-browser export fixture holds the production lease
  until the next durable generation, then prepares the same selected checkpoint.
  One original UI request selected generation 15, advanced through 16, and saved
  generation 15 with matching filename, response headers, exact outgoing SHA-256
  and independently decoded role/save roots. A fresh-origin one-click download
  instead issued two requests: generation 14 closed unfinished, then generation
  16 completed. Its saved bytes exactly match the second response but retained
  the first filename. This reproduces the label mismatch after production response
  headers; the browser/tool's internal mechanism remains unverified. It does not
  retroactively trace the earlier anomaly. Compact traces and decoded identities
  are retained as `browser-export-boundary-a3f2140-20261002.json`; the reusable
  fixture is `scripts/stage7/browser-archive-boundary.ts`. Strict standalone
  TypeScript and focused ESLint passed. Task servers, tabs, copied databases and
  duplicate downloads were stopped/removed. Other concurrent archive lifecycle
  cases, fresh-origin filename handling, precise comparative browser heap and
  final acceptance remain open.

- 2026-10-02 Real HTTP regressions cancel exports before and after response
  headers, confirm server close precedes completion, preserve every Rust
  metadata row and exact managed-source SHA-256, and admit a fresh successful
  export after releasing each lease and temporary file. A separate opt-in
  connected non-reading download used the retained 89,509,376-byte P2 archive:
  the unchanged production idle deadline closed it after 60.03 seconds without
  progress, released its lease and cleaned export artifacts within two seconds.
  The game stayed healthy and advanced from generation 13 to 14; the original
  archive SHA-256 remained unchanged. All three real-server checks, TypeScript
  and focused ESLint passed. The idle case is reproduced with
  `SLITHER_FULL_DOWNLOAD_TIMEOUT_TEST=1`, `SLITHER_DOWNLOAD_TIMEOUT_ARCHIVE`
  pointing to a verified save over 50 MiB, and the named test in
  `server/rustServer.native.test.ts`. Test servers and copied databases were
  closed/removed. This is Windows HTTP failure evidence; real-browser failure
  UI, remaining failure boundaries and complete A1–A10 acceptance remain open.

- 2026-10-02 The real browser Import control uploaded a task-owned damaged
  89,509,376-byte save as its original File and showed the HTTP 400 logical-role
  hash rejection. A repeat at the supported 0.1x live speed kept both samples
  within generation 21: all ten Rust metadata tables, 28 immutable managed-file
  hashes and world epoch stayed identical, with no remaining import scratch.
  A valid original-File retry followed the actual older-save confirmation and
  restored generation 13 into a new branch. The prior generation-21 current
  pointer, all 20 source history rows and 19 source Hall-of-Fame rows remained
  hash-identical. The original archive checksum stayed unchanged. Compact
  request/alert/receipt and source-preservation evidence plus the restored view
  are retained as `browser-import-*-4f05fbd-20261002.*`; the read-only reusable
  audit is `scripts/stage7/import-failure-audit.ts`. Strict standalone TypeScript
  and focused ESLint passed. The test server/tab, copied database, damaged copy
  and scratch were stopped/removed. This proves one real-browser corruption
  case and safe recovery, not precise heap, complete failure coverage or final
  acceptance.

- 2026-10-02 The unchanged production Rust runtime generated paired fresh and
  generation-13 P0/P2/P3 saves with normal physics, sensors, inference and
  evolution active. A bounded independent reader verified all eight logical
  role hashes and each save root, rebuilt each compressed numeric candidate
  byte-for-byte using the production level/window settings, and confirmed the
  strictly-smaller adaptive selection. Exact streamed JavaScript decimal array
  sizes put complete archive reductions at 5.91–8.70x; the large P2/P3 fixtures
  were 5.97–5.98x, above the four-times limit. Raw/candidate/selected bytes,
  container overhead, bounded block maxima and independent Python codec
  throughput are retained in `archive-codec-d6d138b-20261002.json`. Existing raw
  fallback and reusable-scratch Rust test evidence applies to the unchanged
  native source. Strict standalone TypeScript, focused ESLint and decimal
  counter edge/rejection checks passed. All three disposable servers/databases
  and the new archive/metric scratch were stopped/removed after retaining the
  compact report. These eight-second fixture rounds and Python timings are
  size/codec evidence, not Rust/Oxygen timing, total phase-memory or precise
  browser-heap acceptance; the remaining Stage 7/8 gates stay open.

- 2026-10-02 Added opt-in bounded archive-phase intervals to real Rust jobs
  (`SLITHER_TRACE_ARCHIVE_PHASES=1`), including nested decode, validation,
  publication and candidate construction. Ordinary jobs omit the trace; the
  4,096-record cap never interrupts archive work. Export now drops its fully
  validated source population before assembly and post-write validation instead
  of retaining two restored populations together. Exact real-socket roundtrips
  passed with profiling both disabled and enabled; 471 Rust tests plus the
  compile-fail doctest, 108 native/MT tests, TypeScript, ESLint, Vite, rustfmt
  and Clippy passed. A separate production child process imported and exported
  an evolved 90,862,592-byte P2 save with identical SHA-256; 96 memory samples
  and the worker-request boundaries are retained in
  `archive-phases-4cd2287-20261002.json`. Health p95 was 18.32 ms. Short phases
  without a matched sample remain explicitly unknown. This is preparatory
  phase evidence, not total I/O overhead, all legacy readers, player latency or
  complete A4 acceptance. A rejected foreign-build probe exposed premature
  Hall-of-Fame publication during import validation; fixing that rejection
  cleanup is the next slice. Disposable profile processes/databases/exports
  were stopped/removed; only compact reports are retained.

- 2026-10-02 Rust save imports now rebuild Hall-of-Fame objects and their
  inventory inside an operation-owned validation directory. Build/state
  rejection, extraction panic, private construction and known object-collision
  checks precede permanent publication; recursive cleanup owns only the newly
  created private stage. Existing digest objects remain shared and untouched.
  An evolved-save regression exercises foreign-build rejection and extraction
  panic with and without a pre-existing elite, preserving the original upload.
  Real HTTP import/retry/export preserves evolved elite bytes; a corrupt shared
  elite rejects without changing any Rust metadata row, managed-file hash or
  live world epoch, and restoring the task-owned elite allows export again.
  All 472 Rust tests plus the compile-fail doctest and 24 real-server tests
  passed, as did TypeScript, focused ESLint, rustfmt and Clippy. Startup
  scavenging now recognizes stale private import stages under the existing
  24-hour grace; all three scavenger tests and direct Node filesystem checks
  preserved permanent finals, recent stages/children, unknown contents, nested
  directories and junction targets. Test processes and temporary roots were
  closed/removed. This closes
  the observed validation-rejection orphan case; partial final-publication and
  later metadata-commit failure cleanup still need their complete A7 audit.

- 2026-10-02 Failed import and Reset/New Run publication now schedules
  reference-checked orphan reclamation in the SQLite worker. A confirmed
  transaction rollback reclaims files while Rust still holds the old world,
  before cancellation resumes it. A failure before staging defers cleanup to
  the next durable generation boundary with no other publisher active. Live
  export/elite leases or invalid retained references prevent deletion; an
  unknown commit outcome still faults and preserves committed evidence.
  Real SQLite activation triggers rejected evolved-save import and New Run
  transactions after permanent files were present. Every Rust metadata row,
  managed-file hash and old world identity remained unchanged after cleanup;
  valid retries succeeded. A separate rejected-stage case independently
  reclaimed its candidate at the next generation boundary. Worker tests cover
  leases, missing retained files and invalid-response termination. The focused
  persistence/generation/server set passed 86 tests with nine existing
  conditional skips; TypeScript, ESLint and Vite passed. Rust is unchanged from
  the preceding 472-test checkpoint. Disposable servers and fixture roots were
  closed/removed. These prove the named rollback/deferred cases; the full A7
  failure matrix and remaining Stage 7/8 acceptance still require their audit.

- 2026-10-02 A real import failure injected after the first permanent file rename
  exposed Windows publication replacing a pre-existing immutable file through
  `std::fs::rename`. Windows now calls `MoveFileExW` without replacement or
  cross-volume copy flags; canonical parents retain long Unicode path support
  and interior NUL is rejected before mutation. Sequential and concurrent
  different-byte candidates prove one unchanged retained destination and an
  intact rejected source. Evolved-save tests exercise ordinary I/O failure and
  panic with absent/shared checkpoint/shared elite destinations: private scratch
  disappears, shared files and the original upload remain byte-identical, and
  valid retries restore the exact state. The real background task rejects before
  candidate installation; ordinary errors leave its runtime healthy while panic
  faults it. New unreferenced finals remain for the already-tested worker's safe
  boundary/restart reclamation.
  All 477 Rust tests plus the compile-fail doctest, 110 native/MT contracts and
  57 persistence/scavenger tests passed; three existing conditional cases were
  skipped across the JavaScript runs. Rustfmt and all-target Clippy passed.
  Prior TypeScript/ESLint/Vite evidence applies to the unchanged browser and
  Node sources. Test roots/processes and the isolated panic-addon copy were
  closed/removed. This proves the named late-publication failures and Windows
  no-replacement invariant; complete A7 and remaining Stage 7/8 acceptance stay
  open.

- 2026-10-02 Upload failure cleanup now owns only files it successfully created.
  Regressions first reproduced deletion of a pre-existing partial and replacement
  of a pre-existing ready file. Publication now reserves the ready name with
  exclusive creation after the complete partial is synced, then renames over
  that owned reservation. Both collision cases preserve existing bytes; a
  concurrent duplicate operation cannot remove or interrupt the first writer.
  The focused upload/disk/scavenger/real-server set passed 41 tests, including
  the actual 60-second connected-upload idle deadline, with only the optional
  large-download case skipped. That case then passed separately using a new
  evolved P2 save generated by the actual Rust runtime: 90,862,592 bytes, SHA-256
  `1e0c0549a7656d5985e96099eb1d58bd55bdfc66790a4cf1064176b058842927`.
  The non-reading response closed without completion after 60,008.87 ms;
  its lease and export scratch were reclaimed within the two-second assertion,
  the game advanced from generation 13 to 34, and the original checksum stayed
  unchanged. The checksum comparison streams the original file. TypeScript,
  ESLint and Vite passed; Rust is unchanged from the preceding 477-test
  checkpoint. Disposable servers/databases and both generated save copies were
  closed/removed after retaining this compact evidence. These prove the named
  ownership and idle-transfer cases; the remaining A7 matrix and Stage 7/8
  gates stay open.

- 2026-10-02 Real HTTP framing coverage rejects over-limit/noncanonical lengths,
  conflicting chunked framing, empty and truncated requests, and a valid save
  followed by bytes beyond its declared length; valid bounded chunked imports
  still succeed. Rejections compare every retained Rust metadata row, managed
  filename/hash, and active identity. An actual 4-GiB-plus-1-MiB chunked upload
  exposed a socket reset before the intended error response. Import now returns
  the request iterator without destroying its socket, discards buffered input
  for one event-loop turn, and sends its rejection with connection closure.
  The full-limit test passed with HTTP 400, a 4,292,501,366-byte observed partial
  spool, unchanged saved state, and continuing simulation; its spool and private
  server/database were removed. The actual 60-second stalled-upload deadline
  also passed. Across framing, spool, real-server and category coverage, 43
  distinct tests passed, with only the separate optional large-download case
  skipped. TypeScript, ESLint and Vite passed. A reset fixture now waits for its
  initial export's actual lease release before submitting Reset. Rust sources
  remain unchanged from the preceding 477-test checkpoint. These prove the
  named wire-framing/limit cases; complete A7 and Stage 7/8 acceptance stay open.

- 2026-10-02 Completed-upload disconnects reproduced unintended import commits
  after spooling, preparation, and staging. Production now checks client/server
  cancellation before preparation and before committing the replacement. A
  prepared candidate first holds the old world so the existing reference-aware
  worker can reclaim its unreferenced files before cancellation releases that
  world. Real HTTP regressions disconnect after spool/preparation/stage and
  immediately before commit, using a distinct supported legacy candidate;
  every Rust metadata row, retained filename/hash and active identity is
  unchanged, new candidate/scratch files disappear, and fixed steps resume.
  Complementary disconnects after actual SQLite commit and after the Rust swap
  finish publication: the new database pointer and live identity agree, retained
  source files remain unchanged, and process restart resumes and exports that
  same committed checkpoint. Across HTTP/server/persistence/scavenger coverage,
  95 distinct tests passed; four existing optional cases were skipped.
  TypeScript, ESLint and Vite passed. Rust and the spool/idle-transfer path are
  unchanged, so the preceding full-size and 60-second deadline evidence applies.
  Disposable servers/databases were closed and test scratch removed. These prove
  the named completed-upload disconnect boundaries; full A7 and Stage 7/8 gates
  remain open.

- 2026-10-02 Real HTTP imports accepted nonzero outer-save padding, damage in
  the second terminal zero block, and hidden filename bytes after NUL. The save
  reader now shares the checkpoint reader's strict header/checksum/type/path,
  duplicate/count, padding and complete-trailer scan, with separate fixed
  role-size policies. Structural validation seeks past payloads; hashing and
  numeric decoding remain streamed. Fifteen real HTTP rejection cases cover
  those defects plus continuous/link/device/sparse/PAX entries, duplicate/unsafe
  paths, missing roles, unsupported version, false decoded size and corrupted
  graph/root bytes. Every retained metadata row, managed filename/hash, active
  identity and original upload source remains unchanged. A Rust regression
  rejects the three previously accepted defects before scratch creation and
  still validates the original evolved archive with Hall of Fame data.
  All 478 Rust tests plus the compile-fail doctest, 138 native/MT contracts,
  Rustfmt, all-target Clippy, TypeScript, ESLint and Vite passed. Three existing
  optional transfer cases were skipped; the unchanged HTTP/spool paths retain
  their preceding full-size and idle-deadline evidence. Disposable test roots
  and processes were closed/removed, including the isolated test-addon copy.
  These prove the named container failures; full A7 and Stage 7/8 remain open.

- 2026-10-02 Stage 7 controller preservation: replacement preparation silently
  discarded player/trainer actions even when the replacement later failed.
  Existing leases now keep control during preparation; a staged replacement
  holds bounded newest input until cancellation resumes the unchanged world,
  or discards it after the durable swap. Real sockets prove cancelled import
  and actual SQLite-rejected Reset/New Run preserve leases, retained metadata
  and files, and apply held steering. Successful counterparts discard held
  actions, reject old tokens, deliver one replacement notice, and steer only
  after explicit rejoin with new tokens. Focused server/routing/receipt tests,
  the mandatory native/MT overlay, TypeScript, ESLint and Vite passed. Rust is
  unchanged from the preceding checkpoint. Disposable test state was removed;
  the remaining A6/A7 matrix and Stage 7/8 gates stay open.

- 2026-10-02 A8 compatibility evidence now covers v2 child rows and gzip or
  embedded populations with missing, null or zero format columns. Actual
  read-only Rust conversion writes a separate durable destination and leaves
  the complete source database byte-identical. Writable startup preserves all
  original population, Hall-of-Fame, player and graph-preset rows. These cases
  and old browser JSON export/re-import through real archive HTTP with the
  original Float32 population digest and unchanged logical archive contents.
  Focused server/archive coverage, TypeScript and ESLint passed; production
  and Rust are unchanged from the preceding fully green CI checkpoint.
  Disposable servers, database copies and scratch were removed. A live v2
  conversion round trip exposed the next defect: its population-only notice
  is omitted from export, the archive kind remains exact-generation-boundary,
  and the imported copy loses that notice. Full A8 and Stage 7/8 remain open.

- 2026-10-02 Fixed the lost legacy-origin notice: new conversions retain bounded,
  versioned source facts inside checkpoint bytes, projected into archive v1 as
  `legacy-population-import`. Import rejects a contradictory kind or origin
  even when outer framing is valid; real HTTP cases preserve all retained rows
  and managed-file hashes. The source seed remains provenance while normal reset
  rules initialize missing state with the new run's seed. Browser JSON, v2 rows,
  and all six gzip/embedded format-column cases preserve the notice through
  export, import and restart. Real later-generation cases retain it after
  evolution, and Reset/New Run clear it. Previous state encoding remains
  readable; origin length is checked before allocation. All 481 Rust tests and
  the compile-fail doctest, 150 native/MT contracts, 12 focused metadata/client
  tests, Rustfmt, all-target Clippy, TypeScript, ESLint and Vite passed. Three
  existing optional transfer cases were skipped. Disposable test roots and the
  isolated test addon were removed. Already exported archives that omitted
  provenance cannot reconstruct it; owner-data inventory and full A8/Stage 7/8
  gates remain open.

- 2026-10-02 A6 production continuation coverage exports actual generation-two
  and generation-four boundaries, restores through HTTP, then compares the next
  two generations with uninterrupted execution, including a process restart.
  A Dense/GRU/LSTM/RRU graph and durable baseline bots produce identical
  successor roots, complete archive bytes, history and retained unique winners
  with one or six restore workers. Gates hold real FULL-commit replies;
  population construction and stepping remain production code. Focused tests,
  category registration, TypeScript and ESLint passed; the preceding broad
  native/source validation applies. Disposable servers and fixtures were removed.
  This proves the named no-external-input continuation cases; complete A6 and
  the remaining Stage 7/8 gates stay open.

- 2026-10-02 A7 real HTTP coverage exposed an accepted identity conflict:
  independently valid saves with the same run and generation but different
  checkpoint roots replaced the current game. The import transaction now rejects
  that conflict, including an explicit branch request, without changing retained
  rows or files. Retrying the failed operation with the genuine older checkpoint
  still creates its requested branch and preserves the source future. Nine
  compressed-population rejection cases cover window/output/block bounds,
  dictionaries, truncation, extra frames and altered decoded bits; a separate
  decoder verifies the 518-byte expansion fixture produces 16 MiB. Real HTTP
  failures preserve active identity, every retained metadata row and managed-file
  hash, original upload bytes, and continued stepping. Affected persistence,
  server, archive and restart-continuation tests, TypeScript and ESLint passed;
  unchanged Rust/browser code retains the preceding broad validation. Disposable
  servers and fixtures were removed. These prove the named failures; full A7
  and Stage 7/8 remain open.

- 2026-10-02 A7 resource-admission HTTP coverage injects scarce temporary-quota,
  SQLite/WAL-allowance and operating-reserve readings before upload, after the
  actual synced upload, and after acquiring an export inventory lease. Production
  admission arithmetic rejects each case; retained rows/files, active identity
  and source bytes stay unchanged, stepping continues, and a later exact export
  succeeds. Rejection before upload intermittently reset Windows TCP before the
  small error arrived. Import now writes the complete length-delimited error
  before a fixed, at-most-one-second discard interval and connection closure.
  Repeated early rejections and a connected unfinished upload receive the entire
  error without spooling. Affected server, framing, spool, admission and scavenger
  tests, TypeScript and ESLint passed. The actual four-GiB-plus-one-MiB upload and
  60-second stalled-upload gates also passed with clean scratch and continuing
  authority. Rust/browser code retains prior broad validation. Disposable servers
  and fixtures were removed. Physical filesystem exhaustion, process-memory
  rejection and the remaining A7/Stage 7/8 acceptance scope remain open.

- 2026-10-03 A9 exact startup now selects any retained checkpoint across runs;
  the anchor verifier no longer rewrites the copied database's active pointer.
  A non-current selection validates the original native payload, then commits a
  separate branch under guards for both the source and observed active pointers.
  Advancement or pruning during validation rejects the transaction. Real prior-run
  generation-two and generation-three cases preserve both source runs' retained
  files and compact history, reproduce complete export bytes, advance, and restart
  on the new branch. The production verifier restores and re-exports every anchor
  in the six-anchor fixture. Corrupt, missing and unknown exact selections remain
  health-only faults without substitution or source mutation. Browser status labels
  an explicitly selected checkpoint separately from corruption recovery. Affected
  persistence, native startup/server/archive/continuation, browser status and test
  registration checks, TypeScript, ESLint and Vite passed. Rust retains preceding
  broad validation. Disposable servers and fixtures were removed. Full configured
  retention-budget acceptance and the remaining A9/Stage 7/8 gates remain open.

- 2026-10-03 A1/A9 production download binding now covers an actual generation-two
  export lease held while nine successor checkpoints and retention passes finish.
  The eligible old source stays byte-identical until download and lease cleanup;
  the next real retention pass removes it while preserving all compact history.
  The completed download matches its pre-advancement archive byte-for-byte, with
  matching filename, manifest and HTTP identities, and passes production import.
  Node previously accepted any safely shaped attachment name from the native
  response. It now binds that name to the leased checkpoint and generation;
  altered scalar names after real archive creation fail before HTTP success,
  clean their operation files and leases, and permit a successful fresh request.
  Affected native server/transport, direct browser path and category checks,
  TypeScript and ESLint passed. Unchanged native/browser sources retain preceding
  broad validation. Disposable servers and fixtures were removed. This proves
  server binding and pruning across advancement; the earlier fresh-origin browser
  retry/filename anomaly, full A1 and remaining Stage 7/8 acceptance stay open.

- 2026-10-03 The browser download investigation now records request Range,
  If-Range and fetch-context headers. A current-source P2 fixture reproduces
  the in-app browser's saved-name mismatch: one UI click selects generation 27,
  its response closes unfinished, and a separate GET supplies generation 120
  under the first name. A plain click without the download-wait helper closes
  its original response immediately after headers; a later helper-assisted
  click saves generation 246 under its initial generation-168 name. Neither
  request pair uses Range or If-Range. Independent bounded decoding validates
  every saved role/root and matches each complete later response's byte hash.
  The duplicate transfer mechanism remains uninspected; rejecting Range requests
  would not address these cases. Precise tab heap reporting works, but forced
  collection is unsupported on this browser surface; two observations prove no
  comparative memory gate. Compact traces and decoded identities are retained
  as `browser-download-trace-6839782-20261003.json`. The evolved Hall-of-Fame
  round-trip test now joins the actual export cleanup reply before hashing
  immutable files. Focused native transport, retained-resume, export-binding,
  Hall-of-Fame and graph-fixture tests, TypeScript and focused lint passed.
  All diagnostic servers, tabs, copied databases and duplicate downloads
  were cleaned; only the named ignored current-source archives remain for the
  unfinished comparative gates. A1/A2/A3 and the remaining Stage 7/8 stay open.

- 2026-10-03 A7 export failure checks reproduced deletion of pre-existing
  Hall-of-Fame scratch, archive partial and ready files. Rust now registers
  cleanup ownership only after exclusive creation succeeds; Node accepts ready
  ownership only after native preparation succeeds. Real HTTP failures preserve
  the colliding bytes, retained metadata/files and active identity, release their
  leases, permit continuing steps and reproduce the complete archive on retry.
  A native evolved fixture also rejects numeric-encoder scratch creation and
  ready publication collisions without changing any prior file; every retry
  reproduces its original export bytes. The full release Rust suite and doc test,
  affected native server/archive/retained-resume and supporting JavaScript tests,
  TypeScript, ESLint, Vite, rustfmt and Clippy passed. Disposable tests cleaned
  their servers and fixtures. The previous browser fixture's three archives were
  removed after the strict native source identity changed; its compact report
  remains. These prove the named creation/publication failures; process-memory
  rejection, physical filesystem exhaustion and remaining A7/Stage 7/8 gates
  stay open.

- 2026-10-03 A7 worker inventory failure checks reproduced deletion of an
  existing partial file and replacement of an existing final file. The SQLite
  worker now owns cleanup only after exclusive creation, including a final-name
  reservation before publication. Generation-two worker fixtures preserve every
  compact metadata row and managed file byte, then acquire and release a valid
  fresh inventory containing history and Hall-of-Fame references. Production
  import now checks cancellation before admission and again before opening its
  upload spool. A real disconnected peer held across disk admission creates no
  spool or replacement, preserves the experiment, permits advancing steps and
  reproduces its complete archive on retry. Malformed-upload checks now prove
  the busy gate cleared with a real export before checking final files; socket
  closure alone did not prove server cleanup finished. Affected worker/HTTP
  transport and related native continuation/export-binding/upload/cleanup/browser
  contracts, TypeScript, full ESLint and Vite passed. Unchanged Rust retains the
  preceding full release suite. Disposable servers and fixtures were removed.
  Physical resource exhaustion, full A7 and remaining Stage 7/8 gates stay open.

- 2026-10-03 A6 ordered-input continuation now uses an actually evolved
  generation-two archive, a Dense/GRU/LSTM/RRU population, durable baseline bots,
  and fresh player/trainer joins through the production native command boundary.
  Direct continuation and imports with one or six calculation workers accept
  identical ordered actions at identical completed steps, deliver bit-identical
  ordered sensor/position events, and publish identical generation records,
  successor checkpoint roots/files and winner-weight files. Reversing player
  steering changes the observed gameplay; every replay preserves the original
  archive bytes. This compares completed-step chronology; process wall debt and
  OS-entropy reconnect tokens are outside the checkpoint replay contract.
  All 483 release Rust tests and the documentation test, rustfmt, production and
  test-inclusive Clippy passed. The production addon was rebuilt and 12 focused
  source-identity, HTTP continuation/export/import/Hall-of-Fame and live controller
  replacement checks passed. Unchanged TypeScript/browser sources retain the
  preceding broad checks. All private fixtures and servers were cleaned. SQLite
  durability and socket rejoining remain separate integrated evidence; this
  closes the named ordered-input comparison, not full A6 or Stage 7/8 acceptance.

- 2026-10-03 A7 native admission checks reproduced an accepted import that
  exceeded its session's supplied memory ceiling. Background transfer and cold
  archive/replacement jobs now retain that ceiling instead of substituting four
  GiB. A real production HTTP fixture independently exports and re-imports a
  valid large-brain archive under the normal budget, then rejects that import and
  an equivalent Reset under a smaller native budget before staging or commit.
  Both failures preserve every prior metadata row, managed file byte and active
  identity, release scratch, and allow further steps; a valid smaller archive
  then imports successfully. The two-snake fixture isolates admission and is not
  a full P2 performance or physical out-of-memory measurement. All 483 release
  Rust tests and the documentation test, rustfmt, test-inclusive Clippy,
  TypeScript, full ESLint and Vite passed. The rebuilt production addon passed
  119 affected native construction/source-identity/server/archive/continuation
  checks, with three existing optional skips. Disposable servers and fixtures
  were cleaned. Physical memory/filesystem exhaustion, full A7 and remaining
  Stage 7/8 gates stay open.

- 2026-10-03 A7 actual filesystem-failure checks pass on Oxygen through the
  production server and source-matched Linux addon. An unprivileged private
  namespace mounts only the test's managed directory; successful production
  disk admission precedes reducing that filesystem's capacity to its used
  blocks. Real ENOSPC during upload spooling, native import preparation and
  export encoding preserves every prior metadata row, managed file byte and
  active identity, cleans transfer scratch, and permits continued steps.
  Player/trainer sockets, assignments and epochs survive, and steering sent
  during the failure reaches the unchanged world. Restoring capacity permits
  the same import/export to complete. The reproducible launcher is
  `scripts/stage7/private-filesystem-failures.sh`; its advertised capacity is
  not allocated, and the server's disk is never filled. All three Linux checks,
  four affected Windows fixture regressions, TypeScript and full ESLint passed.
  Product code is unchanged and retains the preceding full Rust and browser
  build evidence. The namespace fixtures, cache, generated Rust target and
  disposable Oxygen checkout/addon were removed. These cover the named real
  filesystem failures; complete A7 and remaining Stage 7/8 gates stay open.

- 2026-10-03 A7 archive completion and final validation failures now have real
  HTTP/controller checks using an evolved population, history and Hall-of-Fame
  weights. The isolated source-matched test addon rejects the USTAR end-block
  write, truncates its completed private file before the length check, or alters
  one stored role byte before the unchanged full validator. Each fails before
  download headers, preserves every prior Rust metadata row and managed file
  digest plus a retained source archive, cleans operation files/leases, keeps
  player/trainer assignments and epochs, and applies steering sent during the
  export request. A subsequent export reproduces the original bytes. Production
  exposes none of these operation-local controls. All 521 feature-enabled release
  library tests, three binary tests and the documentation test, 16 affected
  production-identity/export-binding and isolated HTTP/panic checks, rustfmt,
  production/test-inclusive Clippy, TypeScript, ESLint and Vite passed. Test-addon
  preparation now builds only the required library. Disposable servers, fixtures
  and the isolated addon were removed. These close the named completion/length/
  full-validation cases; complete A7 and remaining Stage 7/8 gates stay open.

- 2026-10-03 A7 real HTTP import checks now reject correctly hashed archives
  with incompatible explicit Concat input ordering, a missing population record,
  or a missing dense slot. The independent small-archive repacker checks both
  published logical-root formulas against actual exports and successfully imports
  its unchanged control before constructing each semantic failure. Existing Rust
  validation rejects graph identity, index length or dense ordering before any
  replacement staging or persistence commit. An evolved fixture proves every
  prior metadata row, managed file digest, history/Hall-of-Fame entry, controller
  assignment/epoch and retained source archive survives; steering sent during
  upload completion applies and a fresh export reproduces the original bytes.
  The affected transport file passed 63 checks with four existing optional skips;
  all three cases also pass with nonempty evolved history/Hall-of-Fame fixtures.
  Four production-identity/graph compiler checks, TypeScript and focused ESLint
  passed. Production code and Rust source are
  unchanged. Disposable servers and fixtures were removed; complete A7 and
  remaining Stage 7/8 gates stay open.

- 2026-10-03 A7 real download cancellation now covers body delivery and a
  paused client with the actual server response waiting for socket drain.
  Both use an evolved archive above eight MiB with nonempty history and
  Hall-of-Fame weights. The unfinished response closes, its ready file and
  source lease are released, every prior metadata row and managed file digest
  is preserved, and player/trainer assignments and steering remain usable.
  A fresh direct download is byte-identical to the retained original source
  archive and finishes its own lease cleanup. The affected transport file,
  TypeScript and focused ESLint passed; production code and Rust source are
  unchanged. Disposable servers and fixtures were removed. These cover the
  named streaming boundaries; complete A7 and remaining Stage 7/8 gates stay open.

- 2026-10-03 Cancelled downloads were still starting native archive preparation
  after the server had observed the disconnect. Export now checks cancellation
  after source selection and disk admission, releasing the real source lease
  without starting population decode/encoding. Real HTTP regressions reproduce
  both former native calls and now prove neither occurs, while preserving evolved
  metadata/files, history/Hall-of-Fame data, controller leases, steering and a
  retained source archive. A later direct export remains byte-identical. Archive
  transport plus normal roundtrip, source binding and production-addon identity
  checks, TypeScript and focused ESLint passed. Rust is unchanged. Disposable
  servers and fixtures were removed; complete A7 and Stage 7/8 gates stay open.

- 2026-10-03 The P2 large-download boundary case passes through the ordinary
  in-app UI and production Rust server at normal simulation speed. One original
  request selects generation 30, waits until generation 31 is durable, and saves
  a 113,978,368-byte attachment. Independent bounded TAR/Zstandard decoding
  verifies all role hashes and both logical roots; filename, manifest, response
  headers and complete body digest agree with the selected checkpoint. Compact
  evidence is `evidence/stage7/windows-ryzen5800x/browser-download-9b59a0c-20261003.json`.
  A standalone bounded probe reproduces differing plain-click/helper behavior;
  an earlier real 12x download remained unfinished, with cause unproven. This
  closes the named large-download case, not complete A1 or browser memory and
  usability gates. Seven affected evolved-fixture checks, TypeScript and focused
  ESLint pass. Diagnostic processes, tabs and copied databases were removed; one
  validated original browser archive is retained for pending A2/A3 checks.

- 2026-10-03 The reusable archive fixture now includes approved P1: 300 evolved
  snakes with the default 13,458-parameter graph, ten baseline bots and 3,500
  pellets. Its generation-13 archive is 13,964,288 bytes; exact streamed JS
  numeric formatting gives an 84,356,588-byte decimal-JSON lower bound, including
  81,099,508 bytes for population weights alone. Independent bounded decoding
  verifies all payloads and the save root; compact evidence is
  `evidence/stage7/windows-ryzen5800x/p1-archive-bfe20b7-20261003.json`.
  The browser transport fixture also supports a fresh authority and passive
  consumed-byte hashing. A real HTTP upload verifies the original 113,978,368-byte
  P2 file's digest and restores its generation-30 checkpoint. That command-line
  check is not A3 browser acceptance. The bounded browser upload observer is
  prepared; actual browser upload, memory and usability gates remain open.
  Focused existing transfer-path checks, TypeScript and ESLint passed. Disposable
  servers/databases and duplicate saves were removed; the P1 save and validated
  P2 browser file are retained for the pending browser checks.

- 2026-10-03 An evolved successful-import check now combines live browser-player
  and trainer sockets, held old input, stale reconnect tokens and two identical
  imports. Each replacement advances the world epoch, emits one state-replaced
  result, keeps valid sockets open, rejects stale ownership and allows an
  explicit fresh join with working steering. Repeated imports preserve every
  metadata row, immutable managed-file digest and original save byte; re-export
  matches the original archive. Existing exact-continuation tests additionally
  compare the next generation and restart with Dense/GRU/LSTM/RRU and one/six
  workers. The combined transport, continuation and routing suites pass
  (72 tests; four opt-in quota/large-upload cases excluded), as do TypeScript
  and focused ESLint. This adds A6 controller-replacement evidence; remaining
  browser and final migration gates stay open.

- 2026-10-04 Opt-in archive diagnostics now record whole-process resident memory
  at each Rust stage's entry/exit and on a requested two-millisecond cadence.
  The job joins its observer on success/unwind; ordinary jobs create none.
  A 90,862,592-byte evolved P2 import/re-export preserves exact bytes and covers
  all 59 stages, including 38 missed by HTTP polling. Passive upload/download
  observations also preserve actual stream byte counts. Desktop peak RSS is
  292.9 MiB, local health p95 is 16.2 ms, and player/protocol-bot action p95 is
  at most 7.51 ms, including a post-response observation tail. The report is
  `evidence/stage7/windows-ryzen5800x/archive-memory-bc14d09-20261004.json`.
  These sampled whole-process readings do not prove isolated archive overhead,
  all legacy readers, the 16 GiB VM or checkpoint-overlap durability. All 485
  Rust tests plus the doctest, 104 affected JS checks (six optional cases
  excluded), rustfmt, Clippy, TypeScript, ESLint and Vite pass. Task-owned
  servers/databases and copied saves were removed after retaining the report.

- 2026-10-04 A bounded production P2 measurement now triggers ordinary exports
  from actual generation-transition events without delaying the checkpoint path.
  Native job-clock brackets and conservative router start/finish bounds prove
  export-source assembly overlaps each of eight complete durability barriers
  by at least 258.8–285.5 ms. Selected source generations 2–9 bind to their
  successor FULL commits; original downloads are 75.9–84.1 MB and their actual
  streamed byte counts/digests are retained. Complete barrier p95/max is
  763.0 ms, local health p95 is 16.7 ms, event-loop p95/p99 is 19.99/21.74 ms,
  and player/protocol-bot applied-input p95 is 7.73/7.91 ms. Both connections
  remain open across reassignment, with zero discarded scheduler time. Evidence
  is `evidence/stage7/windows-ryzen5800x/archive-checkpoint-overlap-7f7be75-20261004.json`.
  Six timing-bound/registration tests, TypeScript, ESLint and Vite pass; Rust
  source is unchanged and reuses the preceding native validation. The exact
  child exited normally and its database, managed files and downloads were
  removed. This adds desktop overlap evidence; 16 GiB VM, isolated archive
  overhead, legacy-reader, physical browser/LAN and final Stage 7/8 gates remain
  open.

- 2026-10-04 The same P2 overlap measurement now runs on Oxygen's 16 GiB KVM
  VM with unchanged 1 GiB operating reserve, 9 GiB temporary quota and 4 GiB
  checkpoint budget, using the separate persistent filesystem. All eight
  source-assembly overlaps are proven, but this target-host run misses the
  one-second barrier p95: 1,103.6 ms, also its maximum, with two samples above
  one second. Health p95 is 4.02 ms, player/protocol-bot input p95 is
  16.07/15.64 ms, and event-loop p95/p99 is 10.94/12.17 ms. There is no
  discarded scheduler time, but simulated/wall progress is 0.9763 during
  concurrent exports. The report and point-sampled host context are
  `evidence/stage7/oxygen-ryzen2700/*overlap*52659f9-20261004.json`.
  First-export source population decoding alone takes 543 ms and repeats
  during final validation. This identifies a concrete codec cost; host
  contention is recorded without attributing the miss to it. The Linux addon
  identity and real archive trace-off/on roundtrip checks pass. All disposable
  processes, database/managed files, checkout, dependency cache and build
  artifacts were removed after the copied report digest was verified. VM
  overlap latency and remaining Stage 7/8 gates stay open.

- 2026-10-04 Compressed population decoding now hashes reconstructed packed
  blocks instead of issuing one SHA update per Float32. One reusable 1 MiB
  scratch buffer is included in admission accounting; exact bits, per-brain
  ownership, block limits and final digest rejection remain checked. All 487
  Rust tests plus the doctest, rustfmt, all-target Clippy and 105 affected real
  server/archive checks pass (six opt-in cases excluded). The rebuilt Linux
  addon passes identity and trace-off/on archive roundtrips. A second eight-
  overlap P2 VM run still fails: barrier p95/max is 1,131.5 ms, six barriers
  exceed one second, simulated/wall progress is 0.9749, and discarded scheduler
  time is 183,667 microseconds. Source numeric-decode mean is 442.0 ms versus
  426.1 ms in the preceding run; these runs do not establish a speed improvement.
  Health p95 is 5.06 ms and player/protocol-bot input p95 is 24.48/21.54 ms.
  Exact-source evidence and paired point-sampled host context are
  `evidence/stage7/oxygen-ryzen2700/*block-hash-e1902f1-20261004.json`.
  The copied report digest matches; the stopped task-owned checkout, generated
  databases, dependencies and build artifacts were removed. The VM performance
  gate and remaining Stage 7/8 requirements remain open.

- 2026-10-04 Raw and compressed numeric restoration now appends packed blocks
  into each already-reserved brain allocation, replacing per-Float32 owner
  division/checks. Misaligned/oversized appends fail before partial writes;
  owner buffers do not grow and special Float32 bits survive block/owner
  boundaries. All 489 Rust tests plus the doctest, rustfmt, all-target Clippy,
  105 affected real server/archive checks (six opt-in cases excluded),
  TypeScript, ESLint and Vite pass. Linux identity and trace-off/on real archive
  roundtrips also pass. Eight P2 overlaps on the 16 GiB VM are proven with
  unchanged disk admission: mean source numeric decoding is 234.2 ms versus
  442.0 ms in the preceding run. Complete barrier p95/max is still 1,068.2 ms,
  with four samples above one second; simulated/wall progress is 0.9754 and
  discarded scheduler time is 4,334 microseconds. Health p95 is 4.58 ms,
  player/protocol-bot input p95 is 22.49/26.75 ms, and event-loop p95/p99 is
  11.00/12.74 ms. Evidence and paired point-sampled host context are
  `evidence/stage7/oxygen-ryzen2700/*packed-append-742842e-20261004.json`.
  The report digest matches its retained copy; the stopped task-owned checkout,
  databases, dependency cache and generated builds were removed. This reduces
  an observed codec cost; the full VM performance and Stage 7/8 gates remain
  open. Next measure four/five/six calculation workers with archive activity
  before selecting the supported configuration.

- 2026-10-04 The P2 overlap runner now accepts four/five/six calculation
  workers, verifies the actual welcome count and records the requested count
  in success/failure evidence. Fresh-checkout data parents are created before
  free-space admission. Sequential four/five-worker measurements each prove
  eight actual overlaps on the 16 GiB VM and share the preceding six-worker
  run's unchanged native source. Four/five/six barrier p95/max is
  1,093.1/1,061.9/1,068.2 ms; simulated/wall progress is
  0.9744/0.9759/0.9754, with 178,334/5,667/4,334 microseconds discarded.
  None meets the complete performance gate. Local health p95 is 4.39–4.58 ms,
  player/protocol-bot input p95 is 22.24–26.75 ms and event-loop p95/p99 remains
  below 11.04/12.78 ms. The default remains five; these point-context runs do
  not establish a worker-count winner under controlled host load. Reports and
  paired context are `evidence/stage7/oxygen-ryzen2700/*workers*-3d31070-20261004.json`;
  six-worker evidence is the preceding packed-append report. Six clock-bound
  and registration tests, invalid-count preflight, TypeScript, ESLint, Linux
  identity and real trace-off/on archive roundtrips pass. Native correctness
  reuses the preceding checkpoint's unchanged-source validation. Both retained
  report digests match, and the stopped task-owned checkout, fixtures, cache
  and generated build artifacts were removed. Worker count alone does not
  clear the VM gate; next inspect the remaining numeric writer/transition cost.

- 2026-10-04 Checkpoint numeric writing now packs complete borrowed Float32
  spans into the caller's buffer, retaining the four-byte pending cell only
  for partial reads. No allocation, format, RNG or durability policy changes.
  Ragged/empty owners, block boundaries, special bits and untouched buffer
  tails are checked. All 491 Rust tests plus the doctest, rustfmt, all-target
  Clippy, 105 affected server/archive checks (six opt-in cases excluded),
  TypeScript, ESLint and Vite pass. Exact import/restart successor comparisons
  now include four/five workers alongside one/six and pass on Windows/Linux;
  Linux identity and trace-off/on archive roundtrips pass. Ten actual P2
  checkpoint/export overlaps with five workers cover 614.95 measured seconds:
  barrier p95/max is 912.75 ms, discarded wall time is zero and overload is
  false. Health p95 is 3.67 ms, player/protocol-bot input p95 is
  20.10/18.17 ms, event-loop p95/p99 is 10.90/11.75 ms and peak process RSS is
  544.5 MB. Playable-generation intervals average 61.26 seconds (max 61.38),
  but simulated/wall progress is 0.979323, below the required 0.98; the complete
  VM gate remains open. Reports and paired point-sampled host context are
  `evidence/stage7/oxygen-ryzen2700/*bulk-writer-92da6d0-20261004.json`.
  The copied report digest matches, and the stopped task-owned checkout,
  fixtures, dependencies/cache and build artifacts were removed. The pause
  gate passes in this run without establishing a controlled causal speedup;
  next profile the remaining terminal evolution/transition cost. Remaining
  Stage 7/8 browser, memory, compatibility, LAN/trainer and final gates stay open.

- 2026-10-04 Bounded production health counters now attribute successful
  terminal steps separately from ordinary slow-step costs and the later Node
  persistence barrier. They report control, physics, successor preparation,
  nested serial evolution, admission and remainder without retaining a step
  history or changing candidate/checkpoint data. Partition, nesting and
  saturation checks pass; real durable-boundary HTTP assertions pass with
  one/four/five/six workers on Windows/Linux. The 491-test production Rust suite,
  530-test feature suite, three benchmark-contract tests, doctests, rustfmt,
  all-target/all-feature Clippy, native identity, telemetry projection,
  TypeScript and ESLint pass. A short five-worker P2 diagnostic proves three
  actual export overlaps over 185.90 measured seconds. Mean terminal cost is
  485.17 ms: preparation 463.23 ms, including serial evolution 462.78 ms;
  admission 13.81 ms; control/physics 5.20/2.80 ms; remainder 0.13 ms.
  Persistence-barrier p95/max is 839.72 ms, dropped wall time is zero and
  simulated/wall progress is 0.97891. This identifies evolution as 95.4% of the
  measured terminal computation, not a ten-minute acceptance pass or a finer
  attribution within evolution. The process-lifetime terminal maximum includes
  warmup. Evidence and paired point context are
  `evidence/stage7/oxygen-ryzen2700/*terminal-profile-d9edda0-20261004.json`.
  The retained report digest matches, and the stopped task-owned Oxygen
  checkout, fixtures, dependencies/cache and generated builds are removed.
  Next inspect evolution's species/statistics and reproduction costs; the full
  VM performance and remaining Stage 7/8 acceptance gates stay open.

- 2026-10-04 Evolution counts statistic weights once per owned span and checks
  far-species rejection once per 256 weights. Floating additions, final RMS
  decisions, RNG draws and population policy remain unchanged. Independent
  scalar comparisons cover threshold neighbors, block boundaries, isolated
  spikes, large owners and exact statistic bits. All 492 production Rust
  tests plus the doctest, rustfmt, all-target Clippy, native source identity,
  exact one/four/five/six-worker archive continuation on Windows/Linux,
  TypeScript and ESLint pass. Ten actual five-worker P2 checkpoint/export
  overlaps cover 614.95 measured seconds: barrier p95/max is 895.11 ms,
  dropped wall time is zero, health p95 is 3.26 ms, local player/protocol-bot
  input p95 is 18.21/16.21 ms, event-loop p95/p99 is 10.88/11.57 ms and peak
  process RSS is 557.0 MB. Nine playable-generation intervals average
  61.25 seconds (max 61.31). Mean terminal computation is 440.06 ms,
  including 419.20 ms of evolution nested within 419.61 ms preparation;
  admission is 13.72 ms. The preceding short diagnostic observed 462.78 ms
  evolution; differing run lengths and point-sampled host load do not prove
  a controlled causal speedup. Simulated/wall progress is 0.979478, below
  the required 0.98, so the complete VM performance gate remains open.
  Reports and paired context are
  `evidence/stage7/oxygen-ryzen2700/*evolution-span-81bc541-20261004.json`.
  The copied report digest matches, and the stopped task-owned Oxygen
  checkout, fixtures, dependencies/cache and generated builds are removed.
  Next separate evolution's statistic/species and reproduction costs before
  further optimization; remaining browser, memory, compatibility, LAN/trainer
  and final Stage 7/8 gates stay open.

- 2026-10-04 Production terminal health now separates statistics/species
  scanning from elite copying and breeding/mutation inside serial evolution.
  These bounded clocks never enter checkpoint data; nested bounds and counter
  saturation are checked. The 531-test feature Rust suite, three benchmark
  contract tests, doctest, rustfmt, all-target/all-feature Clippy, TypeScript,
  ESLint, source identity, telemetry projection and real one/four/five/six-worker
  durable-boundary continuation on Windows/Linux pass. A short five-worker
  P2 diagnostic proves three actual checkpoint/export overlaps over
  186.33 measured seconds. Mean terminal computation is 476.32 ms; evolution
  is 454.27 ms, containing 195.80 ms statistics/species scanning and
  243.90 ms reproduction, with 14.57 ms remaining validation/fitness/sorting
  and other evolution work. Persistence-barrier p95/max is 848.18 ms, dropped
  wall time is zero and simulated/wall progress is 0.978483. This is attribution,
  not a ten-minute acceptance pass. Evidence and paired point context are
  `evidence/stage7/oxygen-ryzen2700/*evolution-profile-55160bf-20261004.json`.
  The retained report digest matches; the stopped task-owned Oxygen checkout,
  fixtures, dependencies/cache and generated builds are removed. Both scanning
  and reproduction are substantial optimization targets. The full target-VM
  performance and remaining Stage 7/8 acceptance gates stay open.

- 2026-10-04 Mutation selection now compares the original Uint32 RNG word
  against `ceil(rate * 2^32)` once computed per graph node, avoiding per-weight
  Float64 conversion. Every original draw, Gaussian calculation and clamp is
  retained. Probability-boundary neighbors, zero/subnormal/maximal rates,
  large mixed GRU/LSTM/RRU owners, exact weight bits and cached/uncached RNG
  continuation match the original floating scan. All 12 RNG checks, the
  534-test feature Rust suite, three benchmark contracts, doctest, rustfmt,
  all-target/all-feature Clippy, native identity and real one/four/five/six-worker
  archive continuation on Windows/Linux pass. TypeScript/ESLint evidence reuses
  the preceding unchanged source. Ten actual five-worker P2 checkpoint/export
  overlaps cover 615.24 measured seconds: barrier p95/max is 982.97 ms,
  dropped wall time is zero, overload is false, health p95 is 3.51 ms,
  local player/protocol-bot input p95 is 18.36/16.19 ms, event-loop p95/p99
  is 10.89/11.59 ms and peak process RSS is 557.1 MB. Nine playable-generation
  intervals average 61.27 seconds (max 61.37). Mean terminal computation is
  443.66 ms; evolution is 421.36 ms, including 175.08 ms statistics/species
  scanning and 232.52 ms reproduction. Simulated/wall progress is 0.979155,
  below 0.98; the full VM gate remains open and these differing-run/host
  measurements do not establish a controlled causal speedup. Reports and
  paired point context are
  `evidence/stage7/oxygen-ryzen2700/*mutation-cutoff-a2ca260-20261004.json`.
  The copied report digest matches; stopped task-owned Oxygen checkout,
  fixtures, dependencies/cache and generated builds are removed. Next examine
  conservative species bounds that can avoid full comparisons without changing
  classification. Remaining Stage 7/8 acceptance gates stay open.

- 2026-10-04 Species classification can now accept clearly close genomes from
  a guarded triangle bound on cached weight norms, computed during the original
  ordered statistics pass. Empty/mismatched/uncertain or very long owners keep
  the scalar distance path; the species cutoff, greedy representative order,
  statistic bits and RNG draws are unchanged. The additional eight bytes per
  population slot are charged in admission. Threshold neighbors, subnormals,
  large/extreme owners and independent scalar summaries pass, as do the
  535-test feature Rust suite, three benchmark contracts, doctest, rustfmt,
  all-target/all-feature Clippy and real one/four/five/six-worker archive
  continuation/source identity on Windows/Linux. TypeScript/ESLint evidence
  reuses the preceding unchanged source. Ten actual five-worker P2 overlaps
  cover 612.67 reported seconds: persistence-barrier p95/max is 946.29 ms,
  discarded wall time is zero, overload is false, health p95 is 3.36 ms,
  local player/protocol-bot input p95 is 16.46/16.81 ms, event-loop p95/p99
  is 10.89/11.64 ms and peak process RSS is 557.6 MB. Nine barrier-finish
  intervals average 61.02 seconds (max 61.31), preserving actual early-death
  transitions. Mean terminal computation is 430.22 ms; evolution is 408.36 ms,
  including 165.05 ms statistics/species and 229.67 ms reproduction.
  Simulated/wall progress is 0.979616, below 0.98; differing external-input
  timing and point host snapshots do not establish a controlled speedup.
  Reports/context are `evidence/stage7/oxygen-ryzen2700/*species-norm-6459de1-20261004.json`.
  Their copied report digest matches; stopped task-owned Oxygen checkout,
  fixtures, dependencies/cache and generated builds are removed. The unchanged
  measurement runner includes sampler shutdown/report aggregation after its
  final counter read and starts timing after the initial health reply; next
  bracket those reads explicitly without reclassifying retained results.
  Ordered player/trainer replay still needs four/five-worker coverage beyond
  its current one/six-worker check. Remaining Stage 7/8 gates stay open.

- 2026-10-04 Completed-step measurements now bracket initial/final health
  requests and use the conservative longest duration for the rate denominator.
  Sampler shutdown/report aggregation are excluded; ten-sample runs require
  at least 600 seconds inside the brackets. Reports retain both bounds and
  separate latency/progress checks; combined success requires both. Nine clock
  tests, including threshold-straddling uncertainty and exact large counters,
  pass on Windows/Linux. Ordered player/trainer archive replay now covers
  one/four/five/six workers on both platforms, matching actions, observations
  and successor checkpoint bytes; changed steering remains a positive control.
  Source identity, rustfmt, all-target/all-feature Clippy, TypeScript and
  affected-file ESLint pass. Unchanged engine behavior reuses the preceding
  535-test feature suite and real HTTP continuation evidence. Ten actual
  five-worker P2 checkpoint/export overlaps span 613.94183–613.94544 seconds:
  36,134 complete steps bound simulated/wall progress to 0.980923–0.980929,
  with zero discarded time and no overload. The conservative rate passes 0.98.
  Nine conservative generation-publication interval upper bounds average
  61.19 seconds (max 61.27). Barrier p95/max is 880.91 ms, health p95 3.48 ms,
  local player/protocol-bot input p95 18.80/17.53 ms, event-loop p95/p99
  10.89/11.70 ms and peak process RSS 557.8 MB. P2's ordinary-step p99 upper
  bucket is 24 ms; its permitted rate/generation gate passes without debt.
  Mean terminal computation is 406.92 ms, including 386.37 ms evolution
  (141.83 ms statistics/species, 230.82 ms reproduction). This is a new bounded
  P2 result, not a reclassification of earlier reports or proof of a causal
  engine speedup. Reports/context are
  `evidence/stage7/oxygen-ryzen2700/*clock-window-445ef44-20261004.json`.
  The retained report digest matches; stopped task-owned Oxygen checkout,
  fixtures, dependencies/cache and generated builds are removed. This passes
  the combined checks for this P2 window; later evolved/soak, remaining
  P0/P1, browser/memory/compatibility/LAN/trainer and final Stage 7/8 gates
  remain open.

- 2026-10-04 The ordinary P0/P1/P2 and trainer-loaded/soak samplers now use
  the same conservative completed-step clock bounds as the overlap runner.
  Final reads begin after the requested duration, and ordinary acceptance
  requires at least 600 seconds with no sampled overload or discarded time.
  Consecutive generation observations retain last-old/first-new publication
  brackets; interval checks use the longest permitted duration and reject
  missed generations. Soak resource samples keep their actual response times
  and observation brackets instead of being retimed at shutdown or failure.
  The loaded report identifies its sampler/summary bytes and rejects edits
  during a window. Five new generation-clock tests and 21 existing clock,
  memory, queue, manifest and native-identity checks pass; script type/lint
  checks pass. A real evolved P0 three-second operational smoke completes
  193 steps with zero debt and correctly leaves its acceptance flag false.
  Both smoke servers are closed and the exact task-owned fixture is removed.
  No engine behavior changes; prior Rust and continuation evidence remains
  applicable. P0/P1 target windows, later evolved/soak and remaining Stage 7/8
  acceptance gates remain open.

- 2026-10-04 A five-worker evolved P1 run with 300 neural snakes, ten baseline
  bots and the owner's actual two-actor PyRL trainer covers
  1,800.02004–1,800.06769 seconds and 107,216 complete steps. Conservative
  simulated/wall progress is 0.992703, with zero discarded time and no sampled
  overload. Thirty generation crossings yield 29 complete publication-interval
  upper bounds averaging 60.96 seconds (max 61.26). The combined gate remains
  false: the process-lifetime step p99 upper bucket is 24 ms, above P1's
  16.667 ms limit; it does not prove an interval-specific after-warm percentile.
  Barrier p95/max is 250/240.89 ms, local health p95 2.68 ms, event-loop
  p95/p99 10.88/11.64 ms and peak RSS 228.2 MB. The ten-minute warm/twenty-minute
  measured RSS check passes at 0.3815 MiB/min and 30.58 MiB above warm median.
  Actual trainer actions increase by 208,142. A separate programmatic player
  receives 57,002 frames, sends 52,649 actions and completes 62 connection
  exchanges: 61 successful reclaims plus one rejected reclaim followed by a
  fresh assignment. Desktop/laptop LAN protocol probes pass the required
  input p95 below 100 ms, including interrupted trials in percentile ranking:
  UI/bot upper bounds are 41.11/43.01 ms and 42.67/50.06 ms respectively.
  These are protocol/frame observations, not browser rendering or loss proof.
  All observed queues remain bounded, with two visible reliable-send failures;
  boundedness alone does not prove complete reliable delivery. Exact-byte
  storage summaries preserve earlier breaches and reject changed/malformed
  limits or unexpected final temporary files; five new tests pass on Windows
  and Linux, plus the two category checks and affected type/lint checks.
  Sampled automatic files peak at 122.17 MB, WAL at 3.02 MB and temporary bytes
  at zero. All ten retained boundaries restore and re-export successfully.
  Final SQLite integrity and metadata/file-size audit passes with no unmanaged
  or temporary artifacts. Reports/context are
  `evidence/stage7/oxygen-ryzen2700/p1-soak-*-d84db47-20261004.json`.
  Retained remote report digests match; disposable trainer/server are stopped
  and task-owned Oxygen checkout, fixtures, dependencies/cache, generated
  builds and desktop client scratch are removed. Unchanged engine behavior
  reuses prior Rust/continuation evidence. Interval-specific P1 p99, P0 target
  windows, later P2/soak, browser and remaining Stage 7/8 gates stay open.
