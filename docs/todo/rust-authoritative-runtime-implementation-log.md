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
| 7 | Acceptance active | Measured P0/P1/P2 server timing, real trainer traffic, large archives and bounded persistence pass; browser drawing/heap, fault supervision and final durability acceptance remain. |
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
