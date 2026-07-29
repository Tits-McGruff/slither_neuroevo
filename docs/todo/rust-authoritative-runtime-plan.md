# Forward plan: Rust-owned Slither runtime

## Document control

**Status:** Approved for implementation by the owner on 2026-07-29.

**Revision:** `2026-07-29-draft-4`.

**Created:** 2026-07-23; substantially revised 2026-07-29.

**Supersedes:** `2026-07-23-draft-1`, `2026-07-28-draft-2`, and
`2026-07-28-draft-3`. Do not implement any earlier draft.

**Current-source audit basis:** The uploaded source tree was identified by an
earlier Git checkout as `exclusive-server-mode-refactor` at `46c2f63`, plus
this uncommitted planning document. The ZIP used for the current source audit
did not itself contain `.git`, a user database, a real save archive, retained
benchmark output, or retained compression fixtures. The current code and
documents can therefore prove current structural behavior; historical commit
claims and prior numerical observations are labelled separately below.

**Implementation status:** Stage 1 reference repairs and correction fixtures
are implemented and recorded below. The 30 Hz versus 60 Hz browser-player
cadence measurement remains open for Stage 2; no later-stage exit gate is
claimed.

The owner explicitly approved revision `2026-07-29-draft-4` in the 2026-07-29
conversation. Commit `7971ed2ddbda86891c77def31d980aedf96b4236` contains the
exact plan file as reviewed and approved. This later status update records the
approval without changing that technical revision. The separate factual
implementation log records the same approval and commit; no earlier draft is
approved.

**Draft 4 planning-turn boundary:** Before approval, revising this plan was the
only intended repository change; no implementation file was changed. The
owner's later approval ended that hold and now authorizes implementation
through the staged evidence and exit gates below.

**Migration direction:** Forward from the current branch. This plan does not
use `git revert`, `git reset`, or restoration of an older tree as its
implementation method.

**Reason for a new plan:** The current recovery plan and architecture decision
incorrectly record kernel-only Rust as an owner-approved restriction. The
owner has directly corrected that record. This draft therefore treats the
owner's requirements stated in the current conversation as the requirements
to plan around, while treating every new technical choice below as a proposal
until it is reviewed.

Material later changes to product behavior, architecture, compatibility, data
retention, destructive maintenance, or user-visible rules require review.
Ordinary implementation details may follow evidence without being falsely
described as owner choices. No document may say that the owner selected,
authored, approved, rejected, or locked a choice unless an actual owner message
supports that exact statement.

## Plain-language summary

The finished program should have three parts:

1. The browser displays the game, collects mouse and UI input, and shows
   charts and settings. It does not run the game.
2. A small TypeScript server accepts browser and RL-trainer connections,
   checks messages, serves files, maintains small SQLite metadata/history, and
   routes server-managed checkpoint/archive files.
3. Rust owns and runs the actual game: time, snakes, pellets, sensing, every
   neural network, recurrent memory, movement, collision, scoring, breeding,
   generations, and the compact pictures of the world sent to browsers.

Rust will run the game on its own background thread and use a persistent pool
of Rust worker threads for work that can safely happen in parallel. Node will
remain free to receive player and RL input while a game step is being
calculated.

The TypeScript/Rust crossing will carry coarse commands and completed output
packets. It will not cross once per snake, once per neural layer, or thousands
of times per second.

The existing TypeScript game remains temporarily as a reference while the Rust
version is built beside it. It will not remain as a second production game
after the Rust path passes the required correctness, performance, LAN,
browser, RL, and persistence tests.

## What the main terms mean

- **Authoritative game:** The one copy of game state whose decisions count.
  There must not be one real world in TypeScript and another real world in
  Rust.
- **Fixed step:** One complete, constant-size slice of game time. At the
  current default of 60 steps per second, one step represents 1/60 second.
- **Engine:** The Rust code and state that run the authoritative game.
- **Node:** The server-side JavaScript/TypeScript process. In the target design
  it handles connections, file serving, and database access, not game loops.
- **Native bridge:** The small connection between Node and the Rust engine.
- **Worker:** A persistent Rust calculation thread. It is not a Node worker
  running another TypeScript brain.
- **Heterogeneous population:** A population in which every snake has its own
  weights and recurrent state. This is the real neuroevolution workload.
- **Stable observation boundary:** All due controllers see the same world
  before any snake moves for that step.
- **Wall time:** Real elapsed time outside the game.
- **Simulation time:** Time advanced by completed fixed game steps.
- **Grace period:** A real-time wait during which a disconnected player or RL
  trainer keeps ownership of its snake and can reclaim it.
- **Parity test:** A test that gives TypeScript and Rust the same input and
  compares their results. A known TypeScript bug is not preserved merely to
  make a parity test pass.
- **Cutover:** The point at which normal production startup uses the completed
  Rust engine as the sole game runtime.

## Requirements stated directly by the owner

The following are requirements from the current conversation, not guesses
made by this plan:

- The repository must be repaired forward from its current state, not reverted.
- The deployed program runs on a Debian VM on an Unraid server.
- The VM has eight allocated Ryzen 7 2700 hardware threads and 16 GB of RAM.
- Laptop and desktop browsers connect to the VM over the trusted home LAN.
- A separate RL trainer on the desktop connects through the server API.
- The heavy game as a whole belongs in Rust. “Game” means the complete
  high-frequency runtime, not only one file named `World` or `SimCore`.
- TypeScript may remain in the browser and in a thin layer between clients,
  Rust, and storage.
- LAN use must remain supported. “Not publicly hosted” means no exposure to
  the public Internet; it does not mean localhost-only.
- Public-Internet hardening is not a priority for this hobby project.
- Many snakes and large brains must not make ordinary play unusable.
- A configured 60-second round at 1x speed must not silently take several real
  minutes on a supported workload.
- Simulation overload must not make snakes phase through each other or corrupt
  neuroevolution selection.
- Player and RL control must remain responsive under load.
- Browser-player steering and boost transmission must not depend on incoming
  sensor messages. It uses one latest desired action with bounded
  change-triggered sends and a measured periodic resend; the RL trainer keeps
  its observation-driven action rule.
- A brief lag or disconnect must not immediately hand a controlled snake to
  its brain.
- The handover wait must be much longer than the historical sub-second
  behavior.
- The project’s existing behavior must be read and mapped before Rust replaces
  it. Rust must not be invented from scratch while ignoring the TypeScript
  implementation.
- Rust must not import or reproduce TypeScript-only machinery that has no
  purpose in a Rust-owned engine.
- No implementation work begins until this detailed plan has been reviewed.
- The browser must not parse, reconstruct, or rewrite population save files
  during import or export.
- Export must be an ordinary direct download of one packed save archive
  produced by the server, with each large payload compressed only when doing
  so reduces its measured size.
- Import must upload the selected archive directly to the server without
  browser-side parsing.
- Large numeric population data must remain packed binary data and must not be
  converted to decimal JSON arrays for transfer.
- Import and export must use bounded memory in both the browser and server.
- Live persistence storage must remain bounded during overnight operation
  through packed/adaptive checkpoint files and the explicit retention policy.
- Supported legacy compressed saves must remain importable within documented
  safety limits.
- The selected retention counts and milestone interval are configurable;
  material changes, history downsampling, and deletion outside the selected
  unpinned classes require explicit review.
- A normal export must not create a file that the normal import route rejects
  solely because it exceeds a generic JSON request limit.

## Forward-only repair rule

This plan deliberately keeps useful work already present on the branch:

- the browser UI and renderer;
- Protocol 2 message validation;
- the HTTP API;
- trusted-LAN configuration and launch behavior;
- SQLite transactional persistence and current/legacy compatibility readers;
- run identity, settings, graph editing, Hall of Fame, import/export, and God
  Mode surfaces;
- useful deterministic, protocol, persistence, serializer, and browser tests;
- the current Rust arithmetic where it proves correct and is useful inside
  the future engine.

It does not restore the old broad Rust attempt wholesale. A prior Git-history
review reported that `native/src/lib.rs` at commit `8330065` used hard-coded
defaults, incomplete sensors, simplified head-only collision, a fixed `0.016`
delta, per-call allocations, and behavior that did not match the game. The
uploaded source tree cannot independently prove that historical description.
Stages 1–2 must retain the exact `git show`/diff output, file contents, command,
and commit identity before relying on it. If reproduced, that history is a
warning, not an implementation to resurrect.

Every migration commit will be applied on top of the current branch. Removing
obsolete runtime code happens only after its replacement is working and
verified.

## Verified current-state findings

These findings came from a read-only audit. Evidence is classified honestly:

- **Current-source proof** means the behavior follows directly from files in
  the audited source tree.
- **Git-history evidence** means an earlier Git checkout reportedly established
  the claim, but the audit ZIP cannot reproduce it without the named commit.
  Stages 1–2 must retain the command, commit identity, diff/file contents, and
  raw output before implementation relies on it.
- **Prior planning measurement** means a numerical observation was reported
  earlier but its database/save/fixture/script/raw output/environment is not
  retained in the audited tree. Stage 2 must reproduce it or update every
  dependent estimate and assumption.
- **Derived arithmetic** means a transparent calculation from stated source
  constants or explicitly labelled provisional inputs; it is not benchmark
  evidence.

“Proven” below means current-source proof unless the status says otherwise.
Performance loss that still needs measurement is labelled as such.

### GOV-001: False architecture history

**Status:** Current document contents are proven. The commits that first
introduced the false claims are Git-history evidence awaiting retained command
output.

- `docs/todo/project-recovery-plan.md` calls itself owner-approved and says the
  owner locked kernel-only Rust.
- It says the owner explicitly rejected the broad Rust direction.
- `AGENTS.md`, `README.md`,
  `docs/decisions/0001-native-kernels-and-threading.md`, and the warning added
  to `docs/todo/native_refactor_plan.md` reinforce that claim.
- A prior Git-history review attributed the first combined claims to named
  commits. That introduction history is not independently reproduced by the
  uploaded source tree and must not be presented as current-source proof.
- The owner has now directly stated that those claims are false.

**Required repair:** After the exact plan revision is authorized for
implementation, mark the false restriction as invalid, preserve an honest
historical record of what happened, and prevent future agents from treating
the kernel-only boundary as owner policy. Preserve the exact Git commands,
commit identities, diffs, and relevant file contents before stating when the
false history was introduced.

### ARCH-001: Rust is attached at the wrong boundary

**Status:** Proven structural mismatch; end-to-end slowdown still requires a
fair measurement.

- Every evolved population slot has different weights.
- The current native Dense, MLP, GRU, LSTM, and RRU interfaces accept one
  weight buffer for a whole batch and do not accept a weight stride.
- The real production path therefore constructs one `GraphBrain` per
  population slot and runs them separately.
- TypeScript still copies inputs, walks graph nodes, chooses native functions,
  copies outputs, and advances to the next snake.

**Required repair:** Rust must own the graph, all population weight blocks, all
recurrent state, and the whole due population evaluation inside the engine.

### INF-001: Production “batch” calls have a count of one

**Status:** Proven.

- Dense and MLP graph nodes call their native batch functions with count one.
- GRU, LSTM, and RRU production calls also use count one.
- With the current default 55 snakes, 60 Hz control rate, and default
  MLP-GRU-Dense graph, the nominal workload while all population snakes are
  alive is 3,300 graph evaluations and about 9,900 N-API calls per simulated
  second.

**Required repair:** The production engine must execute the complete graph for
all due, differently weighted snakes without crossing into Node between
layers or snakes.

### THR-001: Normal startup leaves most VM CPU capacity unused

**Status:** Proven.

- The normal configuration has current Node multi-thread inference disabled.
- `play.sh` starts the ordinary server without enabling it.
- Normal execution therefore runs the TypeScript game and serial native calls
  on one Node simulation thread.

**Required repair:** The Rust engine must use a bounded persistent Rust worker
pool by default on the eight-thread VM, while leaving capacity for Node and the
operating system.

### THR-002: The optional Node worker pool adds avoidable work

**Status:** Proven for `--mt`.

- The parent copies inputs and indices into shared storage, clears outputs,
  broadcasts one command to every worker, waits for every worker, and copies
  outputs back.
- Every worker scans the whole submitted batch, filters entries using modulo,
  performs a map lookup, walks a TypeScript graph, and makes count-one native
  calls.
- Persistent buffers mean weights are not recopied every step, but the
  per-step orchestration remains.

**Required repair:** Retire this as a production path after the Rust engine
passes cutover gates. Rust workers receive deterministic ranges or jobs
inside one process and write to preassigned result slots.

### SCH-001: The scheduler can enter an overload spiral

**Status:** Proven.

- `SimCore.update()` may perform up to 120 fixed steps in one pump.
- Frames and stats are sent only after the entire pump returns.
- In serial mode, Node cannot process inbound socket messages during that
  synchronous batch.
- Time spent calculating one slow batch is included in the next wall-time
  delta, which creates more due catch-up steps. A slow step can therefore
  produce increasingly large batches.
- A 120-step batch represents two simulated seconds and can occupy many real
  seconds on an overloaded machine.
- `lastTickAt` is set before optional worker initialization and is not reset in
  `start()`, so initialization delay can become first-pump debt.

**Required repair:** Scheduling moves off Node. Commands are checked before
every fixed step. Catch-up never prevents network input from being accepted,
and excess backlog is reported rather than silently creating an unbounded
latency spiral.

### TIME-001: Slow hardware throughput stretches rounds in real time

**Status:** Proven consequence.

- Generation time advances only when a complete fixed step commits.
- If the server completes only 20 steps per real second, a configured
  60-second generation takes roughly three real minutes.
- The fixed delta itself is not enlarged, so raw scheduler lag does not by
  itself skip collision calculations.

**Required repair:** The agreed supported workloads must sustain the required
step rate on the target VM. Unsupported overload must be visible and must
remain responsive and correct.

### CTRL-001: Socket loss causes immediate brain takeover

**Status:** Proven.

- A WebSocket close immediately releases the controller mapping.
- `spawnExternalSnake()` creates an ordinary neural-mode snake.
- On the next step without a controller mapping, the snake’s random brain is
  eligible.
- The browser clears its assigned snake and reconnects as a spectator rather
  than reclaiming the prior snake.

**Required repair:** Add a wall-clock controller lease, reconnect token, and
automatic reclaim. No brain is allowed during the grace period.

### CTRL-002: The older sub-second takeover bug was real

**Status:** Git-history finding reported from an earlier checkout; not
independently reproducible from the uploaded source tree.

- Commit `3989d26` was previously reported to neutralize input after ten
  simulation ticks and release control to the brain after twenty.
- At 1x that was about one third of a second; at high simulation speed it
  could expire in only a few tens of wall-clock milliseconds.
- A catch-up batch could cross the whole timeout before a client reply was
  processed.

**Required repair:** Never restore simulation-tick-based ownership expiry.
Input staleness and disconnect grace use wall time and are separate concepts.
Before citing this history as verified, Stage 1 or 2 preserves the exact
commands, commit/file contents, diff, and raw output that establish the
ten-tick and twenty-tick behavior.

### CTRL-003: Critical control messages can be silently dropped

**Status:** Proven.

- `WsHub.sendJsonTo()` skips any targeted JSON message if the socket’s shared
  buffered amount exceeds 512 KiB.
- That includes `assign`, not only replaceable sensor updates.
- A controller can be reassigned server-side while the client never learns
  the new snake ID. Later sensor packets are ignored by the client and actions
  for the old ID are ignored by the server.
- A browser receives large binary world frames on the same socket, so display
  backlog can cause control messages to be skipped.

**Required repair:** Lifecycle messages are reliable and observable.
Player input, Protocol 2 RL control, errors, assignment, and reclaim use the
priority path. Only replaceable status and display snapshots are coalesced.
None can crowd out assignment or reclaim events.

### CTRL-004: Catch-up batches reuse stale player/RL input

**Status:** Proven.

- Controller registry tick state is updated once before a whole scheduler
  pump, not once before each inner fixed step.
- Sensor messages can be generated for several inner steps before Node gets a
  chance to accept their replies.
- The old held steering input is reused across the batch.
- At high simulation speed, the current 120-actions-per-wall-second limit can
  also be lower than the number of observations emitted.

**Required repair:** Rust drains the newest accepted action before every fixed
step. Rate limiting distinguishes abuse from an intentionally accelerated
controller stream and never treats a whole catch-up slice as one tick.

### CTRL-005: A stationary mouse target becomes stale as the camera moves

**Status:** Proven.

- The browser converts the pointer to an absolute world coordinate only when
  the mouse moves.
- It reuses that world coordinate for later sensor replies.
- When the followed camera moves, especially after a frame stall, a stationary
  cursor no longer represents a stable direction on screen.

**Required repair:** Retain screen-space cursor position and calculate the
desired direction using current player/camera data when each action is sent.

### CTRL-006: Browser-player commands depend on incoming sensor delivery

**Status:** Proven current-source defect.

- `src/main.ts::sendPlayerAction()` constructs and sends the browser-player
  command.
- Its only production call is inside the WebSocket `onSensors` callback.
- Mouse movement updates only the locally stored pointer position.
- Mouse press/release update only local pointer and boost state.
- There is no independent browser-player command timer, immediate boost
  press/release send, or send directly caused by pointer movement.
- If sensor messages are delayed or stop, fresh steering and boost changes
  remain in the browser and never reach the server. Correcting stale absolute
  coordinates alone does not correct this transmission defect.

**Required repair:** Browser-player action production is independent of sensor
delivery. The browser keeps one latest desired command; pointer and button
changes update it; meaningful changes, especially boost press/release, request
an immediate rate-limited send; and ownership activates a periodic latest-value
resend. Stage 2 measures 30 Hz and 60 Hz rather than locking a cadence in this
draft. Direction is recalculated at send time from the retained screen-space
pointer and newest camera/player state. Old unsent actions are replaced, not
queued. Sensor or display stalls cannot prevent steering changes or boost
release from leaving the browser. The server drains the newest accepted player
action before each eligible fixed step. Protocol 2 RL action production remains
observation-driven and is not forced onto the browser-player cadence.

**Final proof:** Stages 1, 5, 6A, and 7 run the browser/server integration test
that suppresses or delays sensors while pointer and boost state change, proving
fresh steering and boost release leave the browser, reach the server, and can
affect the next eligible fixed step.

### SENSE-001: Body sensing is broken in the real game

**Status:** Proven and critical.

- `src/sensors.ts` expects a collision grid with a `map` and a `query()` that
  returns an array.
- The production `FlatSpatialHash` has no `map`; its `query()` accepts a
  callback and returns nothing.
- The real nearby-segment iterator therefore returns before reading a single
  body segment.
- `nearest_body_dist_norm` always reports no nearby body.
- Every body-hazard bin reports clear.
- Evolved brains, baseline bots, browser players, and the RL API all consume
  these false-clear channels.
- Existing sensor unit tests inject an old fake grid shape, which conceals the
  production mismatch.

**Required repair:** Correct the behavior oracle immediately after plan
authorization, then implement the same tested v3 sensor contract in Rust using
the real Rust spatial index.

### COLL-001: The collision index silently omits bodies

**Status:** Proven.

- The collision grid is hard-capped at 200,000 segment entries.
- Once full, `add()` silently returns.
- Later segments are absent from collision and sensor queries.
- Grid reset leaves object references above the current count, which can
  retain dead snakes, bodies, brains, and genomes longer than necessary.

**Required repair:** Rust uses capacity checked against the actual supported
state, grows within a memory budget, and fails a reset/configuration request
clearly before starting if required memory is unsafe. It never silently makes
segments non-collidable.

### COLL-002: Collision outcome depends on snake array order

**Status:** Proven.

- Collision resolution marks a snake dead while iterating.
- A later snake then ignores that now-dead snake.
- Reversing array order can reverse a simultaneous collision outcome.
- Elites are placed first in the next generation, so the bias can
  systematically affect selection.

**Required repair:** Detect collisions from one immutable post-movement state,
then apply all deaths and awards in a separate deterministic commit.

### COLL-003: Dense generations can spawn already intersecting

**Status:** Proven.

- Each snake independently chooses a random position and heading inside 60% of
  the arena radius.
- There is no head or body separation check.
- Large populations can begin in collision and die immediately.

**Required repair:** Deterministic, collision-safe spawn placement with bounded
attempts and an explicit failure or deterministic fallback when a requested
configuration physically cannot fit.

### COLL-004: Collision settings and high-speed safety are incomplete

**Status:** Proven configuration mismatch; tunnelling risk depends on settings.

- `collision.neighborRange` is configurable but the current resolver always
  checks one surrounding cell.
- Segments are indexed only by midpoint, not every cell crossed by the segment.
- Default fixed substeps are reasonably small, but supported higher speeds and
  larger substep settings are not covered by swept-crossing tests.

No actual speed/body/substep tunnelling threshold is claimed as measured by
this draft. Stage 2/5 must retain the fixture, configuration, raw result, and
environment for any threshold it reports.

**Required repair:** Index the cells covered by segment bounds, honor the
configured range where relevant, and test swept high-speed crossings.

### RNG-001: Joining as a player or RL bot changes evolution randomness

**Status:** Proven.

- `spawnExternalSnake()` draws a random genome from the same evolution RNG
  later used for breeding and mutation.
- It draws spawn geometry from the same world RNG used by normal game
  construction.
- Merely joining therefore changes future evolution and world random
  continuation before considering intentional physical interaction.

**Required repair:** External controllers have a separate versioned RNG stream
or do not have a neural genome until an explicit later takeover.

### OBS-001: Controller types do not currently see one stable boundary

**Status:** Proven.

- Current `World.step()` samples external-controller sensors before due
  baseline respawns.
- It then respawns baseline snakes and samples neural/baseline controls.
- External controllers and neural/baseline controllers can therefore observe
  different membership at what is presented as the same pre-movement step.

**Required repair:** The Rust transaction samples every due controller from one
defined post-respawn, pre-movement state. This is an intentional correction,
not a TypeScript parity target, and fixtures must prevent the old split
visibility from being golden-mastered.

### GRAPH-001: Saved weight order depends on JavaScript locale sorting

**Status:** Proven compatibility risk; the actual saved-ID population must be
  inventoried in Stage 2.

- The current graph compiler uses JavaScript `localeCompare()` for graph-key
  ordering, incoming-edge ordering, and topological tie ordering.
- Node IDs are not restricted to a bytewise-safe subset.
- Ordinary Rust string/byte sorting is not equivalent, and locale behavior can
  differ between Windows and Debian.
- Node order determines parameter offsets and architecture keys, so a naive
  Rust sort can attach saved weights to the wrong nodes without changing the
  total parameter count.
- Incoming-edge order can also change Concat feature order, which changes the
  meaning of downstream weight columns even if whole node blocks are remapped.

This proves a compatibility risk, not that a retained real owner save has
already produced different Windows and Debian orders. Stage 2 must preserve
the actual graph/save fixture and both environment outputs before making that
stronger claim.

**Required repair:** Preserve explicit legacy ordering evidence and migrate to
a versioned locale-independent canonical order. A current checkpoint/preset is
accepted only when its legacy layout can be proven and each node weight block
and incoming feature order can be mapped by stable identity/explicit port;
otherwise it is rejected
non-destructively with a clear compatibility error. Rust must never guess an
order or silently scramble saved weights.

### FRAME-001: Full-world frames can overload the LAN client

**Status:** Proven scaling behavior; exact breakpoint requires measurement.

- Every display frame allocates and serializes every point of every alive
  snake and every pellet.
- The same full frame can be sent up to 30 times per wall-clock second.
- Large bodies can therefore move substantial allocation, LAN, parsing, and
  rendering work onto the laptop even after the simulation itself moves to
  Rust.

**Required repair:** Rust first preserves serializer v1 for compatibility and
Node keeps only the newest unsent display frame. Lifecycle and control messages
have a separate priority path. A negotiated culled/detail-reduced frame is
post-cutover work unless measured v1 traffic prevents the supported LAN
latency target. RL bot connections continue to receive no world frames.

### FRAME-002: Welcome metadata needlessly serializes the complete world

**Status:** Proven current-source allocation/integration defect; lower priority
than the authoritative-loop and control defects.

- `server/simServer.ts::refreshWelcomeState()` calls
  `WorldSerializer.serialize(this.core.world)` only to read the resulting
  buffer's `byteLength`.
- That performs another complete world traversal and allocation outside
  ordinary frame publication.
- Welcome refresh runs around startup and runtime reconstruction/configuration
  paths, so large worlds can make startup, Reset, New Run, import, settings
  refreshes, and connection-related welcome work more expensive.

**Required repair:** This does not require a new protocol. Once Rust packs
frame v1, the Rust engine or frame-routing layer exposes the latest packed
frame byte length as small metadata. Node must not serialize, reconstruct, or
traverse the authoritative world merely to calculate a welcome field.

**Final proof:** Stage 6 adds an integration test proving a welcome-state
refresh reads cached/latest frame-length metadata and does not invoke another
complete world serialization.

### ID-001: Resurrected snake IDs cannot be represented exactly in frame v1

**Status:** Proven.

- Serializer v1 stores every snake ID in a `Float32Array`.
- Consecutive integers are exact only through 16,777,216.
- The current resurrected-snake allocator starts at 1,000,000,000.
- At that magnitude, multiple consecutive internal IDs round to the same
  Float32 value, so browser selection/God Mode can address the wrong identity.

**Required repair:** The scalable serializer carries exact integer identity.
During v1 compatibility, Rust must use a checked wire-safe mapping/range and
must never silently round two live identities to the same browser ID.

### PERF-001: Existing performance tests do not test the production path

**Status:** Proven.

- The world performance test runs a JavaScript `World` without the production
  native bridge or worker pool.
- Native kernel tests use 256 observations sharing one weights buffer.
- They do not measure different genomes, recurrent state, graph traversal,
  worker dispatch, sensing, movement, collision, frames, Node responsiveness,
  memory, or achieved simulation/wall-time ratio.
- Broad absolute timing limits can pass even if the integrated native path is
  slower than JavaScript.

**Required repair:** Add end-to-end workload comparisons on the real
heterogeneous population and run the production acceptance matrix on the
Debian VM.

### FAULT-001: Release panics currently abort the whole in-process server

**Status:** Proven.

- `native/Cargo.toml` sets `[profile.release] panic = "abort"`.
- The target engine runs inside the Node process through N-API. A Rust panic
  under that profile terminates Node immediately; it cannot become a live
  engine-fault result while `/health` remains available.
- A caught engine/worker panic and an unrecoverable process crash therefore need
  separate, honest recovery contracts.

**Required repair:** Stage 3 changes the release panic strategy to unwind and
catches the coordinator/N-API roots that exist in the minimum spine. Each later
calculation/archive/persistence thread adds its root catch when introduced. A
caught panic becomes one faulted-engine result and never unwinds across FFI.
Native memory faults, aborts, double panics, and process-level OOM still require
service restart from the latest valid checkpoint; the plan does not promise
live Node health after the process itself dies.

**Final proof:** Stage 7 release-build panic injection covers coordinator,
calculation worker, archive worker, persistence boundary and synchronous N-API
entry roots, then kills the process during representative operations and proves
external service restart chooses only a valid durable boundary.

### PERSIST-001: Browser export materializes and rewrites the whole population

**Status:** Proven.

- `src/main.ts::exportServerSnapshot` calls `response.json()` for the complete
  `/api/export/latest` response.
- `src/storage.ts::exportJsonToFile` then pretty-prints the complete object,
  creates a population-sized browser `Blob`, and starts the download.
- The browser therefore holds the parsed population, a second complete JSON
  string, and the downloadable bytes at the same time.
- The browser also substitutes local UI settings, graph, and Hall-of-Fame data
  instead of downloading one authoritative server-produced save.

**Required repair:** Export becomes one ordinary direct download of one
server-produced packed adaptive binary archive. Browser JavaScript never parses,
reconstructs, stringifies, or creates a Blob for population data.

**Final proof:** Stage 7 archive acceptance tests A1, A2, A5, A6, and A10.

### PERSIST-002: Browser import duplicates the file and hits a JSON limit

**Status:** Proven.

- `src/storage.ts::importFromFile` calls `FileReader.readAsText()` and
  `JSON.parse()` on the complete selected file.
- `src/main.ts::importServerSnapshot` constructs another population object,
  calls `JSON.stringify()`, and sends it through `/api/import`.
- `server/httpApi.ts::readJsonBody` buffers all chunks, concatenates them,
  creates one UTF-8 string, and parses the entire body.
- The generic body limit is 50 MiB even though otherwise valid populations can
  be much larger.

**Required repair:** The selected `File` is uploaded unchanged to a dedicated
binary route. The server spools and validates it with separate archive-byte,
declared-decoded, record, and memory-admission limits. Browser JavaScript
never reads or parses the archive.

**Final proof:** Stage 7 archive acceptance tests A1, A3, A4, A7, A9, and A10.

### PERSIST-003: Full automatic snapshots grow SQLite without bound

**Status:** Proven.

- The default is a full population checkpoint every generation.
- Every current snapshot writes one uncompressed Float32 weight BLOB per
  population slot, with no delta, deduplication, compression, retention, or
  pruning.
- `--fresh` deliberately preserves prior rows, manual save adds another full
  snapshot, and Hall-of-Fame storage adds a decimal-JSON genome each
  generation.
- SQLite has no production snapshot deletion path, retention policy, vacuum
  policy, or code-level WAL growth control.

**Required repair:** New checkpoints use managed packed adaptive files rather
than population-sized SQLite rows. Automatic snapshots follow the selected
count/milestone/byte policy, and file pruning plus legacy SQLite maintenance is
measured on an overnight-sized fixture. Pinned data and downloaded exports are
never automatically deleted.

**Final proof:** Stage 7 archive acceptance test A9 and the combined
database/managed-store growth budget.

### PERSIST-004: Decimal JSON recreates the failure compression had addressed

**Status:** Proven structure. The exact expansion figure is a prior planning
measurement pending artifact-backed reproduction.

- `typedGenomeToJson()` calls `Array.from(weights)`, turning four-byte Float32
  values into decimal JavaScript numbers and text.
- The current server exporter stringifies those arrays one genome at a time,
  after which the browser materializes the complete result.
- Decimal Float32 JSON necessarily uses more than the four packed bytes for
  ordinary values and adds array punctuation plus browser object/string
  overhead.
- A prior planning measurement reported about 20.03 text bytes per Float32,
  roughly five times packed bytes, but its fixture/script/raw output and
  environment are not retained in the audited tree. Stage 2 must reproduce or
  replace that number.
- The older `genomes_blob` path was gzip-compressed, but the current transfer
  format discarded compression while retaining JSON expansion.

**Required repair:** All new transfer and at-rest population weights remain
lossless packed little-endian Float32 binary. Each payload selects raw packed
or fixed-block byte-shuffle plus Zstandard according to actual size. JSON is
permitted only for small structured metadata.

**Final proof:** Stage 7 archive acceptance tests A5, A6, and A8.

### PERSIST-005: Server response backpressure does not bound browser memory

**Status:** Proven.

- `sendJsonChunks()` waits for Node response-drain events and
  `exportSnapshotJsonChunks()` iterates database genomes.
- That bounds part of the server path only.
- `response.json()` still buffers and parses the complete HTTP entity before
  the browser can use it, and the subsequent `JSON.stringify()` and `Blob`
  create more population-sized browser allocations.

**Required repair:** Treat server backpressure and browser materialization as
separate concerns. The normal browser path delegates the archive response to
the browser's download system from the first byte.

**Final proof:** Stage 7 archive acceptance tests A1, A2, A4, and A10 plus a
static forbidden-call check.

### PERSIST-006: Current transfer drops authoritative state and is not atomic

**Status:** Proven.

- Checkpoint metadata stores best fitness, fitness history, and the pending
  Hall-of-Fame entry, but the JSON transfer type and current exporter omit
  them.
- Export replaces server graph/settings/update fields with browser values and
  attaches browser-local Hall-of-Fame data.
- Import can replace the browser Hall of Fame and reset graph/settings before
  population validation or import succeeds.
- A failed import can therefore leave UI/server state changed even though the
  population was not replaced.

**Required repair:** The archive has one explicit full-experiment contract.
Graph, settings, history, Hall of Fame, run state, and population are validated
as one staged operation. Nothing becomes current, durable, or visible until
the whole import commits at a safe boundary.

**Final proof:** Stage 7 archive acceptance tests A6 and A7.

### PERSIST-007: History and Hall-of-Fame storage repeat large genomes

**Status:** Proven.

- Fitness history is only eight numeric values per generation and is currently
  capped in memory, so the summary itself is cheap.
- Generation checkpoint metadata embeds a decimal-JSON copy of the best
  genome, and the Hall-of-Fame table stores the same best genome as decimal
  JSON again.
- Using the prior provisional 20.03-byte observation, a 402,914-weight brain
  would be about 7.7 MiB as decimal weight text. That is an estimate pending
  Stage 2 reproduction, not an audited measurement. Structurally, the duplicate
  can dominate metadata and can hit the source-defined 16 MiB parent metadata
  limit before the advertised per-genome weight limit is reached.

**Required repair:** Preserve full compact generation summaries separately
from checkpoints. Store Hall-of-Fame metadata separately and reference
deduplicated packed adaptive genome data. Do not repeat a genome in every
checkpoint metadata object.

**Final proof:** Stage 7 archive acceptance tests A5, A6, and A9.

## Current-state evidence index

Current-source rows refer to the audited source tree reported by the earlier
checkout as revision `46c2f63`. The ZIP itself had no Git history or numerical
artifacts. Rows labelled Git-history evidence are not current-ZIP proofs;
Stages 1–2 must preserve the exact command/output/commit/file evidence. Prior
planning measurements are likewise provisional until their named fixtures,
scripts, raw outputs, and environments are retained. This index makes those
boundaries explicit before implementation relies on them.

| Finding | Evidence kind | Source paths, symbols, or commits |
|---|---|---|
| GOV-001 | Current documents plus separately labelled Git-history claim | Current false text in `docs/todo/project-recovery-plan.md`, `docs/todo/native_refactor_plan.md`, `AGENTS.md`, `README.md`, and ADR 0001; prior history review named `3fe62d0` and `258ac69`, whose exact command/diff/file output must be retained; the owner's direct correction in this planning conversation |
| ARCH-001, INF-001 | Code path | `native/index.d.ts`; `native/src/simd_kernels.rs`; `src/brains/graph/runtime.ts`; `src/brains/ops.ts`; `server/worker/inferWorker.ts`; `server/simServer.ts::packPopulationWeights` |
| THR-001 | Configuration/startup | `server/config.ts::DEFAULT_CONFIG.mtEnabled`; `play.sh`; `server/index.ts` |
| THR-002 | Code path | `server/brainPool.ts::runBatch`; `server/worker/inferWorker.ts::runInfer` |
| SCH-001, TIME-001 | Code path | `src/sim/SimCore.ts::update`; `server/simServer.ts::start` and pump/frame/stat publication |
| CTRL-001 | Code path | `server/wsHub.ts` close handler; `server/simServer.ts::handleDisconnect`; `server/controllerRegistry.ts::releaseConnection`; `src/world.ts` control selection; `src/main.ts` reconnect path |
| CTRL-002 | Git-history evidence requiring retained output | Prior review named `3989d26:server/config.ts` default `actionTimeoutTicks = 10` and `3989d26:server/controllerRegistry.ts::getInputForSnake` release at twice that value; Stage 1/2 retains exact commands/diffs/file contents |
| CTRL-003 | Code path | `server/wsHub.ts::sendJsonTo` and shared socket `bufferedAmount` limit |
| CTRL-004 | Code path | `src/sim/SimCore.ts::update`; `server/controllerRegistry.ts`; `server/simServer.ts` scheduler pump |
| CTRL-005 | Browser code path | `src/main.ts` pointer-to-world update and action construction |
| CTRL-006 | Browser send call graph | `src/main.ts::sendPlayerAction`; its sole production caller `onSensors`; pointer/mouse/boost handlers that only mutate local state; absence of an independent player-send timer |
| SENSE-001 | Interface mismatch | `src/sensors.ts` legacy-grid adapter versus `src/spatialHash.ts::FlatSpatialHash.query` |
| COLL-001–004 | Code/config path | `src/spatialHash.ts::FlatSpatialHash`; collision/spawn paths in `src/world.ts`; collision settings in `src/config.ts` |
| RNG-001 | RNG call path | `src/world.ts::spawnExternalSnake` and world/evolution RNG ownership |
| OBS-001 | Ordering trace | external sensor, baseline respawn, and neural/baseline control order in `src/world.ts::step` |
| GRAPH-001 | Compatibility path | `src/brains/graph/compiler.ts` graph key, incoming-edge, and topo `localeCompare` sorts |
| FRAME-001 | Serialization path | `src/serializer.ts`; `server/simServer.ts` frame cadence; `server/wsHub.ts` binary broadcast |
| FRAME-002 | Allocation/call path | `server/simServer.ts::refreshWelcomeState` calls `WorldSerializer.serialize(this.core.world).byteLength` |
| ID-001 | Numeric contract | `src/serializer.ts` Float32 ID field; resurrected allocator in `src/world.ts`; IEEE-754 Float32 exact-integer bound |
| PERF-001 | Test-path proof | `server/performance.test.ts`; default backend construction in `src/world.ts` |
| FAULT-001 | Build/runtime contract | `native/Cargo.toml` release `panic = "abort"`; in-process N-API engine target |
| PERSIST-001, PERSIST-005 | Browser/server export path | `src/main.ts::exportServerSnapshot`; `src/storage.ts::exportJsonToFile`; `server/httpApi.ts::sendJsonChunks`; `server/persistence.ts::exportSnapshotJsonChunks` |
| PERSIST-002 | Browser/server import path | `src/storage.ts::importFromFile`; `src/main.ts::importServerSnapshot`; `server/httpApi.ts::readJsonBody` and the 50 MiB generic limit |
| PERSIST-003 | Schema/cadence | `server/config.ts::DEFAULT_CONFIG.checkpointEveryGenerations`; `population_snapshots`/`snapshot_genomes` schema and save transaction in `server/persistence.ts`; no production prune/vacuum path |
| PERSIST-004 | Numeric transfer encoding | `server/snapshotTypes.ts::typedGenomeToJson`; current JSON exporter; bounded gzip `genomes_blob` compatibility reader |
| PERSIST-006 | Transfer semantics/order | checkpoint metadata in `server/snapshotTypes.ts`; exporter omissions in `server/persistence.ts`; browser substitution and pre-import mutation in `src/main.ts` |
| PERSIST-007 | History/HoF duplication | generation history and `_lastHoFEntry` in `src/world.ts`; checkpoint metadata in `server/checkpoint.ts`; `hof_entries.genome_json` in `server/persistence.ts` |

## Goals

- Make Rust the sole production owner of the whole high-frequency game.
- Keep browser rendering and UI work in the browser.
- Keep Node as a small, responsive LAN/API/database bridge.
- Use the VM’s CPU allocation effectively without starving networking.
- Keep steady memory safely within the VM’s 16 GB allocation.
- Preserve all intended gameplay, evolution, settings, graph, persistence,
  browser, LAN, and RL features.
- Correct the verified sensing, control, scheduler, collision, spawn, and RNG
  defects rather than cementing them as “parity.”
- Make 1x simulation time track wall time for the agreed supported workloads.
- Make overload visible, bounded, and correct rather than allowing silent
  slow-motion, stale control, missing collisions, or lost assignments.
- Make external control reclaimable after transient LAN or browser failures.
- Make save download and upload ordinary browser file transfers whose
  JavaScript heap use does not scale with population size.
- Keep checkpoints, Hall-of-Fame genomes, and full generation summaries
  compact enough for unattended overnight operation.
- Retain supported existing save formats without letting legacy limits or
  generic JSON limits constrain the new archive.
- Give future contributors an honest source of truth that distinguishes owner
  requirements, proposed engineering choices, test evidence, and completed
  work.

## Non-goals

- Reverting the branch or discarding all useful current work.
- Restoring the old incomplete Rust `World` implementation.
- Running a second production game in TypeScript.
- Moving browser DOM, canvas rendering, settings forms, graph editing, or
  charts into Rust.
- Making the hobby server safe for public-Internet exposure.
- Adding accounts, public authentication, TLS termination, cloud deployment,
  or multi-tenant isolation.
- Promising that every possible maximum slider value can be combined and still
  run at 1x on eight Ryzen 2700 threads. A measured supported envelope will be
  published.
- Promising bit-identical floating-point results across different CPU
  architectures, compiler versions, or math implementations.
- Deleting the TypeScript reference before migration/stabilization is complete,
  or treating later destructive deletion as a required deliverable.
- Calling a partial Rust layer or a benchmark of isolated kernels “done.”

## Target ownership

| Area | Final owner | Notes |
|---|---|---|
| HTML, CSS, controls, graph editor, charts | Browser TypeScript | Compiled and served to LAN clients. |
| Canvas rendering, local pointer handling, and player-action transmission | Browser TypeScript | Uses server display frames, keeps one latest desired player command, and sends it independently of sensor delivery; never owns game state. |
| WebSocket and HTTP listener | Node TypeScript | Validates and routes messages without stepping the game. |
| Trusted-LAN bind and URL discovery | Node/Vite configuration | `0.0.0.0` and explicit LAN addresses remain supported. |
| SQLite and legacy snapshot readers | Dedicated Node persistence worker | Owns the synchronous `better-sqlite3` connection; main event loop never performs large statements/chunk loops. |
| Graph preset and Hall-of-Fame database endpoints | Node TypeScript | Rust emits/accepts compact data needed by live game actions. |
| Fixed-step scheduler and backlog state | Rust | Runs away from Node’s event loop. |
| Controller assignment, leases, grace, and current actions | Rust | Node supplies connection events and routes Rust results. |
| World, snakes, bodies, pellets, and scores | Rust | One authoritative copy. |
| Camera, pointer, and presentation view state | Browser TypeScript | Node routes bounded view requests; Rust retains only the latest minimal per-client view data needed to pack/cull a frame. Camera smoothing and display-only behavior do not become authoritative game state. |
| RNG streams and generated IDs | Rust | Versioned and checkpointed. |
| Sensors and spatial queries | Rust | Includes complete v3 contract and corrected body sensing. |
| Neural graph compilation and validation | Rust | Browser validation remains a convenience, not authority. |
| Per-brain weights and recurrent state | Rust | Evolved brains use stable population slots; external/resurrected brains use separate stable brain handles. |
| Complete heterogeneous brain inference | Rust | Evolved/external/resurrected due brains run with their own weights/state; no Node crossing inside a graph or due-brain pass. |
| Movement, boost, food, growth, death, collision | Rust | Complete physics transaction. |
| Baseline bots | Rust | Strategies use the same corrected sensors. |
| Fitness, selection, crossover, mutation, generations | Rust | Uses versioned deterministic streams. |
| Binary display-frame packing | Rust | Exact v1 first; negotiated culled/LOD format later. |
| Stats, sensor, assignment, command-result event creation | Rust | Node only routes them. |
| Checkpoint/archive state construction | Rust | Rust publishes immutable managed checkpoint files; the persistence worker commits small metadata/current-pointer transactions; main Node routes opaque archive files. |

## Target runtime flow

```text
Laptop/desktop browser                 Desktop RL trainer
  - display and UI                       - policy/training process
  - pointer and settings                 - Protocol 2 bot connection
             \                              /
              \        trusted LAN         /
               v                          v
        Node TypeScript interface process
          - HTTP/WebSocket validation
          - static browser files
          - connection IDs and routing
          - asynchronous file response routing
          - small jobs to persistence worker
          - no World and no game loop
                 |                 |
                 |                 v
                 |       Node persistence worker
                 |         - owns better-sqlite3
                 |         - bounded chunk/legacy work
                 |         - no game state
                       |
                       | short nonblocking queue operations
                       v
             Rust engine coordinator
          - wall clock and fixed steps
          - command/action drain each step
          - sole authoritative state
                       |
                       | deterministic parallel jobs
                       v
              Rust calculation workers
          - sensing + complete brains
          - movement proposals
          - collision queries
                       |
                       v
          Rust completed-event queues
          - reliable control/lifecycle events
          - mode-aware controller observations
          - latest replaceable stats
          - latest binary display frames
          - checkpoint requests
                       |
                       v
        Node routes packets / persistence worker commits SQLite
```

## Native bridge design

The bridge is intentionally small. The following is the proposed minimum
semantic surface, not a claim that the owner selected these exact exports,
queue counts, or data structures. Evidence may justify an implementation
revision if it preserves the owner requirements, keeps the boundary coarse,
and does not change external or gameplay behavior. A material architecture or
product change requires review; gameplay work follows the owner decisions
recorded later in this plan.

### Lifecycle

- Create an engine from fully normalized settings, graph, run identity,
  thread count, and optional checkpoint.
- Start the coordinator and worker pool.
- Query a small health snapshot without copying the world.
- Request an orderly stop and wait for worker termination.
- Treat a Rust panic or coordinator exit as a server fault; do not continue
  with an unseen TypeScript fallback. This live fault path applies only to a
  panic caught under the Stage 3 unwind boundaries; an actual process
  abort/native fault is recovered by process supervision and checkpoint
  restart.

### Inbound queue

Node forwards bounded command batches containing:

- connection opened/closed;
- join, spectate, reclaim, and view subscription;
- player/RL actions;
- visualization enable/disable;
- atomic live settings;
- Reset and New Run;
- God Mode kill/move;
- import and resurrection requests;
- correlated Node send results for controller observations/lifecycle events,
  carrying event sequence plus connection and assignment epochs;
- persistence acknowledgement or failure;
- shutdown.

Every command carries the needed transport connection ID, connection epoch,
controller/assignment epoch where applicable, and a monotonic arrival sequence
assigned by Node. Timing uses one elapsed-time domain derived from Rust
`Instant` or a calibrated Node `performance.now()` value; it never uses
`Date.now()` or a civil/system clock that can jump. The engine rejects
regressing timestamps and commands from stale epochs. It validates
game-specific state again even after Node validates the wire shape.

Interactive-player action updates use latest-value semantics per controller.
The browser replaces an unsent player update with its newest state rather than
building a queue; Node/Rust drains the newest accepted update before each
eligible step. Protocol 2 RL action production remains observation-driven and
its ordering remains compatible with the current trainer until a coordinated
later extension is justified. Ordered state changes such as join, disconnect,
reset, settings, and God Mode are never silently overwritten.

### Outbound drain

Node drains bounded batches of already prepared events:

- `assign` and reclaim results;
- controller sensors;
- settings, God Mode, Reset, New Run, import, and resurrection results;
- stats;
- Hall-of-Fame events;
- welcome-state changes;
- binary display buffers targeted to appropriate UI connections;
- checkpoint payloads and commit barriers;
- warnings, overload state, and fatal errors.

Node does not receive a `World`, `Snake[]`, `Pellet[]`, graph object per snake,
or per-layer activation arrays on every step.

After calling `ws.send` for a controller observation or reliable lifecycle
event, Node returns one small accepted/failed result to Rust with the matching
event sequence, connection epoch, and assignment epoch. This is an internal
transport result, not a claim of remote receipt and not a new application-level
acknowledgement protocol. Rust ignores stale, duplicate, or replaced results.

When the combined outbound state changes from empty to non-empty, Rust sends
one coalesced thread-safe wake notification to Node. Further events do not
create more wakeups until Node drains/re-arms the state. The notification
carries no per-snake payload; Node drains a bounded batch from the queue. This
avoids busy polling or a coarse latency timer without turning callbacks into a
new fine-grained game boundary.

Re-arming uses an atomic notified flag plus generation counter. After its final
drain check, Node atomically clears/re-arms the flag and immediately rechecks
both queue emptiness and generation with acquire ordering. If an enqueue raced
between the empty check and re-arm, Node either observes the changed generation
and continues draining or Rust wins the false-to-true transition and sends a
new wake. A bounded drain that leaves work schedules a continuation before
sleeping. No event can remain queued solely because its wake raced with re-arm.

### Event priorities and backpressure

The first Rust cutover needs a small, observable priority model rather than a
second protocol project:

1. **Lifecycle/control:** assignment, reclaim, player input, Protocol 2 RL
   action/observation, command results, persistence barriers, errors, and
   faults. These messages are processed ahead of display traffic and are never
   silently discarded. If a bounded reliable queue cannot accept one, expose
   the fault and close the affected socket while preserving its reclaim grace.
2. **Discrete game events:** generation and Hall-of-Fame events. These are not
   replaceable by a later display frame.
3. **Replaceable status:** only the latest unsent stats update is retained.
4. **Display:** only the newest unsent binary frame per UI connection is
   retained. Replacing an older display frame is expected and does not affect
   simulation or control.

Existing Protocol 2 JSON ordering remains the first compatibility target. New
application acknowledgements, observation coalescing, gap accounting,
lockstep training, binary action batches, and additional trainer modes are
post-cutover work unless the real trainer proves one is required for the first
usable Rust path. Any such change is coordinated with that separate project.

Node always drains lifecycle/control work before admitting another display
frame. It checks the per-socket buffered amount before `ws.send(frame)` and
keeps visual bytes below a measured admission budget. Bytes already handed to
one WebSocket cannot be overtaken, so the v1 maximum-frame case must meet the
control-latency test. If it does not, only the minimum measured display change
needed to meet that gate moves onto the critical path. A large display frame
can never make `assign` disappear.

### Persistence barrier

At a required generation or run boundary:

1. Rust finishes the prior fixed step.
2. Rust creates a separate staged transition. The staged value contains the
   candidate population, cloned-and-advanced RNG state, allocator state,
   run/config identity, history, pending Hall-of-Fame/event state, and every
   other value that would become current.
3. Rust constructs the compact versioned checkpoint/archive stream from that
   staged value without mutating or publishing the prior committed state.
4. Rust emits a correlated checkpoint request and pauses only the transition
   that depends on durability.
5. Rust finishes/fsyncs and atomically publishes the immutable managed
   checkpoint file.
6. The persistence worker commits the small metadata, history/references and
   current pointer in one SQLite transaction.
7. Node acknowledges the exact transition epoch or reports failure.
8. Only on the matching success acknowledgement does Rust atomically make the
   staged run/generation current and release its dependent events.

On failure, the prior committed world and public run/config identity remain
current. The staged transition is retained for an explicit retry or discarded
without recomputing random draws; neither path may consume another draw, leak a
Hall-of-Fame event, publish a success, or partially replace state. Reset, New
Run, import, and generation transition use the same rule.

Capacity planning includes the temporary double memory required for the prior
world plus staged population and bounded checkpoint codec scratch/files. A
reset/import request that cannot fit this bounded staging budget is rejected
before it can become current.

Normal game steps never perform SQLite calls.

## Thread model for the target VM

### Coordinator

One Rust coordinator thread owns all mutable authoritative state and the fixed
step clock. It:

- receives commands without blocking Node;
- drains commands before every fixed step;
- creates immutable views and deterministic jobs;
- waits for its internal workers only inside Rust;
- commits results in a defined order;
- produces outbound events;
- never calls JavaScript from the middle of a game step.

### Worker pool

The final engine creates one persistent bounded worker pool at startup. It
does not spawn operating-system threads per snake or per step.

Stages 3–6 run the new engine with one calculation executor by default. Those
stages define immutable job/result interfaces and reusable scratch. Stage 7
introduces actual multi-worker execution only after the complete scalar engine
passes its correctness and external-contract gates.

No automatic multi-worker default is selected before measurement. The
eight-vCPU VM must run the Node main thread, one Node persistence worker, one
Rust coordinator, and at most one active Rust archive/codec worker in addition
to calculation workers; libuv/runtime helper threads and the guest OS also need
scheduling capacity. Stage 7 therefore compares four, five, and six calculation
workers and may characterize seven. Each result records all runnable threads
and repeats with archive/checkpoint work idle and active. It selects the fastest
count that preserves Node/control/checkpoint latency and does not assume that
filling all vCPUs with calculation workers provides headroom.

V1 permits only one CPU-active Rust archive/codec job at a time. It reports
block progress and yields where the codec permits, while required checkpoint
work retains the priority rules in the persistence section. Additional codec
threads are not enabled merely because Zstandard supports them; they require
the same target-VM calculation/control matrix.

The coordinator may execute small serial commit work while workers handle
large independent ranges. No worker scans work assigned to every other worker.

### Watchdogs for live-but-stuck work

This is a Stage 6B/7 production-hardening contract, not a Stage 3 prerequisite
for sensing or heterogeneous inference.

Size limits alone do not detect a deadlock. Every calculation, archive, and
persistence job therefore exposes an operation ID plus monotonic
started/progress/completed counters that a different thread or process context
can read:

- the proposed calculation no-progress deadline is five seconds;
- the proposed normal archive/persistence no-progress deadline is 60 seconds,
  refreshed after each bounded codec/file/database chunk;
- socket upload/download idle timing is the separate 60-second rule above;
- explicit offline full-database compaction has its own owner-visible deadline
  because one SQLite `VACUUM` cannot be safely preempted as normal game work.

These are provisional configurable Stage 2 starting values, not permanent
limits. Final defaults use observed job progress, decoded state/record sizes,
free disk and VM memory. They are no-progress deadlines, not total
job-duration caps. A hung Rust
worker cannot be safely killed and reused. The watchdog faults health where
possible, stops accepting success, then terminates the process so the required
Debian supervisor can restart from a valid checkpoint. A hung persistence
worker is handled the same way after the parent sees its heartbeat expire.
Injected hangs at calculation, archive-codec, file, persistence-worker, and
WAL-operation boundaries must enter this recovery path instead of freezing
steps, the pending export response, or a durability barrier indefinitely.

### Deterministic parallel work

- Each due snake writes sensors and physical proposals only to its stable snake
  slot; each due graph writes activations/outputs/recurrent state only to its
  stable brain handle.
- Work partitioning is by stable index ranges or an explicitly deterministic
  job list.
- Reductions produce per-snake/per-brain results and are committed in stable
  slot/ID order.
- Random draws that must retain current semantics remain on the coordinator.
- Evolution can remain serial because it happens once per generation and is
  not the hot loop.
- A later per-child RNG scheme may be considered only as a versioned behavior
  change with new replay fixtures.

### Work that is initially serial

Correctness comes before parallelism. The first Rust implementation may keep
the following coordinator-owned while tests are built:

- command ordering;
- ambient random spawning;
- contested pellet award commit;
- death side effects;
- optional explicitly requested visualization target selection;
- evolution and generation transition;
- checkpoint construction order.

Parallelism is added only where state ownership and deterministic commit rules
are clear.

## Rust state and memory layout

The Rust engine must model behavior, not copy TypeScript class shapes
literally. The following layout is proposed because it keeps frequently used
data compact and makes ownership visible.

### Run and configuration state

- normalized run seed and run ID;
- config revision and config content hash;
- protocol, serializer, sensor, RNG, checkpoint, and engine versions;
- complete core and CFG settings;
- compiled graph key and parameter layout;
- fixed-step duration, requested simulation multiplier, wall accumulator, and
  committed tick;
- separate world, evolution, external-controller, and per-baseline RNG streams;
- next external, baseline, and resurrected ID candidates;
- health, fault, overload, and performance counters.

Rust validates every setting range independently. It must not rely on the
browser or Node being the only validator.

### Population and brain state

- one stable dense population slot for each evolved genome;
- one contiguous `f32` weight block per genome using the current compatible
  parameter order;
- one compiled graph shared as immutable structure by compatible genomes;
- recurrent `f32` state laid out by stable brain handle and recurrent node;
- a separate bounded brain slab for non-population snakes that actually own a
  graph, including resurrected snakes and an external snake only if the chosen
  grace-expiry policy gives it neural control;
- an explicit mapping from evolved population slot or non-population snake ID
  to brain handle, with generation/epoch checks so a reused slot cannot inherit
  stale recurrent state;
- reusable per-worker activation and graph scratch;
- optional activation capture for only the selected/focused snake when neural
  visualization is requested;
- population fitness and lineage metadata outside per-frame objects.

There is no `GraphBrain` JavaScript object per population member in production.

### Snake state

Frequently accessed scalar fields are stored in packed arrays or another
measured compact layout:

- stable snake ID and kind;
- alive flag;
- population slot or non-population identity and optional brain handle;
- controller lease identity and control mode;
- position, previous position, direction, radius, speed, and boost;
- age, food, points, kills, target length, and fitness;
- current/previous turn and boost input;
- control accumulator and delivered-observation score boundary;
- body storage range;
- skin/color metadata needed by display frames.

Physics values should begin as `f64` because current JavaScript number
semantics are double precision. Neural weights, activations, recurrent state,
and the existing wire frame remain `f32`. A later physics precision reduction
requires measurements and tolerance tests; it is not assumed to be free.

### Body storage

The current array of `{x, y}` JavaScript objects becomes pooled contiguous
numeric storage. Requirements:

- no object allocation per body point;
- stable per-snake logical order from head to tail;
- growth and shrink without copying every other snake;
- explicit capacity accounting;
- dead-body storage released or reused without retaining genomes and brains;
- no raw pointer kept across a reallocation unless its lifetime is proven;
- a maximum requested allocation checked against the VM memory budget before
  it is committed.

Candidate representations—per-snake vectors with reserved capacity, slab
chunks, or a packed pool with free lists—are benchmarked during Stage 4's
storage/sensing work. The selected representation must pass the same
behavioral tests before its speed is considered.

### Pellet storage

- packed position, value, kind, color, and owner fields;
- stable handles or generation-tagged indices so swap-removal cannot create a
  stale reference;
- reusable capacity;
- a cell index supporting sensing and eating;
- deterministic iteration/claim order;
- no fresh candidate array for every snake in every collision substep.

### Spatial indexes

Collision and sensor queries may share geometric source data but cannot share
incorrect limits.

- Collision indexing has no “ignore the rest” capacity behavior.
- A segment is indexed into every relevant cell touched by its bounds, not
  only its midpoint.
- Query scratch is per worker.
- Duplicate segment hits from multiple cells are suppressed deterministically.
- Sensor work limits apply only to sensor detail, never to collision truth.
- If memory required by a requested configuration exceeds a safe limit, reset
  is rejected with an estimated requirement and no partial world becomes
  current.

### Numeric IDs and frame compatibility

The existing serializer stores IDs in `Float32`, which represents integers
exactly only through 16,777,216. Rust can use wider internal IDs, but it must
retain a checked wire-safe range while serializer v1 is active. ID exhaustion
must produce an explicit transition error rather than rounding two identities
to the same browser value.

The current resurrected allocator begins at 1,000,000,000 and is therefore
already unsafe for v1. The recommended compatibility mechanism is a separate
monotonic public v1 ID in the exact range `1..=16,777,216`, mapped to the
engine's stable `u64` entity handle. All client-facing frame, assignment,
selection, and God Mode surfaces use that public value consistently; physics,
collision ties, population slots, and persistence identities use the internal
handle. Public IDs are not reused within a run, their next value is
checkpointed, and exhaustion rejects the next creation with a plain error
before two identities alias. Reset/New Run/import epoch invalidation prevents
an old client command from crossing into a rebuilt mapping. A later integer-ID
frame removes this compatibility ceiling but is not required for the first
cutover.

### Memory budget

The VM has 16 GB. Initial proposed limits are:

- a hard process safety ceiling below 16 GB so the operating system and Node
  retain headroom;
- a target peak resident set below 8 GiB for normal P0/P1/P2 runs and a hard
  admitted server-process ceiling of 12 GiB for supported stress/import work;
- reject or warn before a reset whose declared maximum body/genome/frame
  storage would push the estimated engine above 12 GB;
- an import admission estimate includes both the current and staged engine and
  rejects before their combined projected process peak exceeds 12 GB;
- archive I/O itself adds no more than 256 MiB beyond current/staged engine
  state and uses disk spooling for transfer;
- the 30-minute and longer soak use the explicit 1 MiB/minute and 64 MiB
  plateau gates in the performance section across repeated generation
  transitions.

These are provisional configurable engineering guardrails, not permanent
limits. Stage 2 records actual decoded state, record counts, free disk, Node,
Rust, browser-frame and persistence memory on the 16 GiB VM before final
defaults are selected.

## Authoritative fixed-step order

Rust must own this complete transaction. The order below is based on the
current `World.step()` behavior, with verified defects explicitly corrected.

### Before the step

1. Read the newest wall clock and update scheduler diagnostics.
2. Drain ordered lifecycle/settings/God Mode commands accepted before this
   boundary.
3. Drain the newest action for every connected or reserved controller.
4. Update controller wall-time state; never expire ownership based on
   simulation tick count.
5. If a required persistence barrier is unresolved, do not begin the
   transition that depends on it.

### Stable pre-movement state

6. Assign the next committed tick ID.
7. Advance generation time, snake age, and survival score by exactly one fixed
   delta.
8. Accumulate and create due ambient pellets using the world RNG.
9. Update baseline-bot respawn timers and create due baseline snakes using
   their own streams.
10. Rebuild or refresh the read-only spatial views required for sensing, and
    ensure `bestPointsThisGen` has been initialized/reset before the first
    sensor sample of a generation, reset, or import.
11. Sample sensors for every due neural, baseline, player, and RL controller
    from this same world state.
12. Calculate each external observation's `points_delta_norm` against its last
    sample accepted by Node for the matching live socket. Creating or replacing
    an unsent observation does not advance that boundary. A send failure leaves
    the delta accumulated and faults/disconnects that controller rather than
    pretending the observation was delivered.

### Control selection

13. Select exactly one source for every alive snake in this priority:

    - baseline strategy for a baseline snake;
    - connected external action for an assigned player/RL snake;
    - held/neutral external action during its lease or grace period;
    - neural graph for an eligible neural snake;
    - explicit neutral control for an external-only snake with no active
      takeover policy.

14. Evaluate all due neural graphs, with different weights and recurrent state,
    wholly inside Rust.
15. Prepare controller sensor events without blocking the step on LAN delivery.
16. Commit every chosen turn/boost only after the complete observation/control
    phase has finished.

### Physics substeps

17. Derive the collision substep count only from the fixed delta and collision
    safety setting, never from `simSpeed` or wall-time lag.
18. For each substep, compute movement/body proposals using the held control.
19. Calculate boost cost and requested boost pellets.
20. Build deterministic food claims against the pellet index.
21. Resolve contested food by nearest eligible head, with exact ties by stable
    snake ID. Apply score/growth changes afterward.
22. Build checked broad-phase bounds covering both previous and proposed
    positions of every moving head and body segment.
23. Detect all wall, relative-motion/swept body, and defined head-head
    collisions from one immutable substep snapshot. A proof that a displacement
    bound makes a simpler test equivalent is acceptable only when captured in
    tests.
24. Commit deaths together in stable order.
25. Award kill score once under the selected killer/tie gameplay rule; no new
    policy is implied merely by requiring deterministic commit.
26. Generate death and boost pellets in stable order using the owning snake’s
    defined RNG stream.

### Finish the step

27. Update best-points state.
28. Record only an explicitly requested visualization target ID if needed.
    Browser camera transforms, smoothing, follow selection, and zoom remain
    presentation state and do not consume a Rust RNG stream.
29. Check early-end and configured generation duration.
30. If the generation ends, compute fitness, history, Hall-of-Fame candidate,
    selection, crossover, mutation, and the next population in their defined
    order.
31. At a resumable boundary, emit the checkpoint and wait for the required
    persistence acknowledgement before constructing random next-world state.
32. Spawn the next generation using collision-safe placement.
33. Commit the tick and publish completed state/events.
34. Make a new latest-only display snapshot only when its wall-clock display
    interval is due.

No frame, stat, or success acknowledgement may describe a partially committed
step.

## Scheduling and real-time behavior

### Fixed-step rules

- The default remains 60 complete steps per simulation second.
- A step always receives the same fixed delta.
- `simSpeed` controls how many fixed steps are requested per wall second; it
  never enlarges physics delta.
- Rendering frequency is independent of simulation step rate.
- Skipping or replacing old display frames is allowed.
- Skipping part of a physics step is not allowed.

### Backlog rules

Because the coordinator is outside Node, it may calculate continuously without
blocking socket acceptance. It still must not create an input-latency spiral.

- Check commands/actions before every step, including every catch-up step.
- Maintain a measured wall-debt value.
- Use a small configurable catch-up horizon, initially proposed as 250 ms.
- When wall debt exceeds the horizon, discard only the excess scheduling debt,
  not any part of an authoritative step, and increment an explicit
  `droppedWallDebt` diagnostic.
- When an interactive player is assigned, do not run a wall-debt catch-up burst.
  After each committed step, give the Node/socket path a bounded servicing
  opportunity before another overdue step. If real-time interactive debt still
  exceeds the horizon, discard the old scheduling request and report overload
  instead of running many steps on stale input.
- The background coordinator checks the inbound queue before every step; Node
  remains free to accept sockets and enqueue new actions while Rust computes.
- Unchanged Protocol 2 RL observations remain bounded and ordered. The engine
  does not silently coalesce them or assume the trainer can answer between
  immediately consecutive accelerated steps.
- Sustained inability to meet 1x enters a visible overload state.
- Overload never changes collision storage capacity, disables sensors, swaps
  controller source, or silently changes the fixed delta.

Discarding excess wall debt prevents an old stall from forcing minutes of
future catch-up. It cannot make an underpowered workload real-time. Required
workloads must instead meet the performance gates.

### Diagnostics

Health/stats gain clear measurements:

- requested simulation multiplier;
- achieved simulated-seconds/wall-seconds ratio over recent and lifetime
  windows;
- current and maximum wall debt;
- dropped wall debt;
- mean, p50, p95, p99, and maximum fixed-step duration;
- time split for commands, sensing, brains, movement/food, spatial build,
  collision, evolution, frame packing, and persistence wait;
- controller action age and queued player/RL observation counts;
- frame bytes, frames replaced, and per-client buffered bytes;
- active Rust worker count and utilization;
- collision/sensor candidate counts and saturation;
- engine, Node, and total process memory where available.

`fps` must no longer be the only performance number because the current value
is merely the latest server pump frequency.

## Controller ownership and player/RL behavior

### Owner-selected state machine

Each external controller has one explicit state:

1. **Connected and active:** newest accepted input is used.
2. **Connected but input stale:** ownership remains; after a short wall-time
   input-hold window, steering becomes neutral and boost turns off.
3. **Disconnected but reserved:** the same snake is reserved for reclaim;
   steering is neutral and no brain runs.
4. **Grace expired:** a configured policy is applied once and the old token
   cannot silently resume control.
5. **Snake dead:** if the connection remains valid, a reliable replacement
   assignment is emitted.

The initial owner-selected configurable values are:

- hold the last accepted input for 500 ms of wall time;
- then use neutral steering/boost-off while ownership remains;
- reserve the snake for 30 seconds of wall time after socket loss;
- expose both timing values through validated configuration;
- never run the snake's brain during the 30-second grace period;
- never expire merely because simulated time is fast or the server emitted
  several steps before a reply arrived.

### Grace-expiry behavior

After the 30-second grace expires, the engine performs one explicit transition
to neural ownership. The expired external assignment/token can no longer apply
input. The brain begins from that transition forward; external and neural
outputs are never mixed and the old controller cannot “fight” the brain.

### Reconnect token

- A successful player/bot assignment includes an opaque resume token.
- The token is an ownership handle, not public-Internet authentication.
- New clients may retain the token from the assignment so the server can bind
  a reconnect to the already-created lease.
- The browser stores it locally and automatically requests reclaim after
  reconnect instead of joining as spectator.
- The RL trainer may persist it for the same run/session.
- A token is bound to server session/run/controller kind and cannot claim an
  unrelated snake.
- Reset/New Run invalidation behavior is explicit in the reclaim response.
- Reclaim within grace returns the same live snake and current sensor state.
- Reclaim after grace returns an explicit expired result and a new assignment
  only when requested.
- The old socket/connection epoch is invalid immediately after reclaim and can
  no longer steer the snake. Assignment/reclaim messages use the priority
  lifecycle path; no new application-acknowledgement protocol is a first
  cutover requirement.

An unchanged Protocol 2 bot remains compatible even if it does not use the new
token. The trusted-LAN fallback may match exactly one reserved lease using a
bounded existing identity key; an ambiguous match is rejected instead of
stealing a snake. Repeated legacy reconnects cannot accumulate unclaimable
leases. The browser in this repository becomes token-aware. Both unchanged and
token-aware bot reconnects are black-box tests.

### Reset, New Run, and import with connected controllers

Every authoritative state has an internal `worldEpoch` that is exact `u64`
control metadata, not a Float32 display ID. Reset, New Run, and import build a
candidate under a new epoch but do not expose it before the durability barrier.
On successful current-pointer/world swap:

- all queued old-world actions, observations, Node send results, assignment
  epochs, and resume tokens become stale atomically;
- an archive never restores live socket IDs, connection epochs, leases, or
  browser/trainer resume tokens from another process/session;
- transport connections may stay open, but a controlled connection enters
  `awaitingRejoin`, receives a priority world/welcome change, and has no
  assignment;
- no action is eligible until an explicit new join/rejoin command has arrived
  in socket order and the server has issued a new assignment epoch/token;
- failure before the swap preserves every old lease/action with the old world
  rather than half-invalidating controllers.

Requiring an ordered rejoin is what makes unchanged Protocol 2 safe: an old
action sent before the rejoin cannot arrive after it on the same TCP stream and
be mistaken for new-world input. The repository browser and trainer-compatible
flow keep sockets open, send one reliable `state replaced` result, and issue no
new assignment until a new ordered join/rejoin arrives. Automatic reassignment
without an action-carried epoch is not offered because it cannot distinguish
delayed old input. Stage 2 records the current behavior, and Stages 5–7 test
this owner-selected behavior with the real browser and trainer.

### Action and observation semantics

- An observation for committed pre-movement boundary `T` produces an action
  eligible for the next not-yet-started step.
- Existing Protocol 2 `action.tick` remains diagnostic-only: the current
  browser sends `T + 1`, while the published bot example sends `T`. Rust must
  not use that ambiguous field to reject or reorder legacy actions.
- Legacy actions are ordered by the server-assigned monotonic arrival sequence
  inside the current connection/assignment epoch.
- The engine records action receipt on the single monotonic elapsed-time clock.
- Late actions never rewrite a committed step.
- For a player, a newer valid action replaces an older unconsumed action from
  the same controller. Protocol 2 RL actions retain their current ordered
  contract.
- The browser remains usable at 1x even if a visual frame is skipped.
- Player input and lifecycle messages have priority over display data.
- RL JSON Protocol 2 remains supported with bounded ordered observations by
  default; it is not silently converted to player-style coalescing.
- A later optional negotiated binary/batched bot-control format may reduce
  JSON array overhead at high `simSpeed`; it is not required for the first
  Rust cutover and cannot remove the existing API without coordination with
  the separate trainer project.

### Browser-player action production

Interactive browser control has a client-side production rule distinct from
the RL trainer:

- while the browser owns a snake, it maintains exactly one latest desired
  player command;
- pointer movement, mouse-button changes, boost press, and boost release update
  that latest state;
- meaningful changes, especially boost press and release, trigger an immediate
  send subject to a small bounded rate limiter;
- a periodic resend remains active for the ownership lifetime so a delayed or
  suppressed sensor stream cannot freeze steering or boost; Stage 2 compares
  30 Hz and 60 Hz on the LAN/browser workloads before selecting the
  configurable initial cadence;
- each send calculates direction from the retained screen-space pointer and
  newest available player/camera state, then sends the newest boost value;
- an unsent older player command is replaced, not queued;
- incoming sensors and display frames may update the data used by a send, but
  neither is the trigger required to make a send occur;
- frame replacement, sensor delay, and sensor suppression cannot prevent a
  player from steering or releasing boost.

Node/Rust retains latest-value player semantics and drains the newest accepted
action before each eligible fixed step. This does not impose a timer on the
separate RL trainer: Protocol 2 RL policies may continue producing an action
in response to each delivered observation.

### Browser steering correction

The browser stores cursor coordinates relative to the canvas, not one old
absolute world target. When sending an action it combines:

- current cursor screen offset;
- latest authoritative player heading/position metadata;
- current follow camera transform.

A stationary cursor at a fixed screen offset therefore requests a stable turn
as the camera follows the snake.

## Sensor contract

### Supported layout

Rust implements the complete current v3 layout:

- 19 scalar values;
- configurable 8–32 bins each for food, body hazard, wall, and other heads;
- total input length `19 + 4 * bins`;
- exact published label order;
- score delta since the snake’s previous delivered observation;
- current normalization, clamping, angles, radii, and boost/length semantics
  unless a correction is explicitly versioned.

Input length, graph input node, welcome sensor spec, baseline strategies,
visualizer, checkpoints, and imported graph validation remain aligned.

### Exact `points_delta_norm` delivery boundary

“Delivered” has one testable meaning for each controller path:

- For an internal neural or baseline controller, delivery occurs when that
  fixed-step observation is consumed by the controller. The snake boundary
  advances once at that point.
- A baseline strategy probe that is not used as its delivered observation is
  pure and does not advance the boundary.
- For an external player/RL stream, remote receipt cannot be known during a
  fixed step. Delivery therefore means Node accepted the observation for
  `ws.send` on the matching live connection/assignment epoch. It does not claim
  TCP or application receipt.
- Replacing an unsent latest-only player sample keeps the previous delivered
  boundary. The replacement retains the accumulated score delta so points are
  not silently lost or double-counted.
- Protocol 2 RL samples remain bounded and ordered. If a send fails, the
  controller faults/disconnects and the undelivered boundary does not advance.
- A stale completion from an old connection, assignment epoch, or replaced
  player observation cannot advance the boundary.

Fixtures cover unsampled intervals, latest-only replacement, ordered delivery,
send failure, and disconnect/reclaim.

### Body-query correction

The intended behavior—not the current broken adapter—is the source of truth:

- nearby body distance reflects real other-snake segments;
- hazard bins reflect body clearance using the configured hit scale;
- baseline bots use the same values;
- controlled player/RL sensor messages contain the same corrected values;
- a production World/engine integration test, not a fake old grid, proves it.

### Sensor work bounds

Sensor caps may protect the engine in an extremely dense area, but:

- they never limit collision correctness;
- candidate order is deterministic and spatially meaningful;
- reaching a cap increments a diagnostic;
- cap behavior cannot default a crowded direction to falsely clear;
- stress tests cover max configured pellet and segment checks;
- per-worker scratch prevents races between parallel snake observations.

## Neural graph and heterogeneous population

Rust must support every current graph node and connection rule:

- Input;
- Dense;
- MLP with hidden sizes;
- GRU;
- LSTM;
- RRU;
- Split;
- Concat;
- explicit ports and graph outputs.

The Rust compiler must independently enforce:

- exactly one valid input contract;
- acyclic ordering where required;
- all edge node/port references;
- Split size sums;
- ordered Concat inputs;
- node input/output size agreement;
- total two-value turn/boost output;
- checked parameter counts and offsets;
- checked weight and recurrent-state lengths;
- finite imported values;
- bounded graph size and allocation.

### Weight compatibility

The migration must preserve what each saved weight means, not merely reproduce
the same total length. The current compiler's `localeCompare()` order is a
compatibility hazard, so the proposed process is:

1. Inventory real checkpoint/preset node IDs and record the current Node
   compiler's explicit node/edge/incoming-Concat order and offsets on supported
   Windows and Debian environments.
2. Define a new graph-layout version with one locale-independent total order
   using lexicographic raw UTF-8 bytes with no Unicode normalization, followed
   by explicit port, edge-kind, and stable structural tie fields. Exact
   duplicate node identities are invalid. Composed and decomposed Unicode IDs
   remain distinct and therefore cannot collapse to one sort key.
3. For a legacy saved graph, verify its stored architecture identity against a
   supported legacy layout and build an explicit node-ID/port block map. Record
   the actual legacy incoming feature order for every multi-input node.
4. Remap each legacy node's packed weights into the canonical Rust layout
   without changing values. A small boundary migration helper may use the old
   TypeScript compiler to describe legacy offsets, but Rust independently
   validates graph structure, block sizes, identities, and the final mapping;
   no TypeScript graph executes in production.
5. Materialize legacy Concat/incoming order as explicit canonical input
   ordinals/ports so feature meaning is unchanged. If an old structure cannot
   express that order directly, the migration must use a versioned graph form
   or explicitly permute every affected downstream input-weight column and
   prove full-graph output parity; moving whole node blocks alone is not
   sufficient.
6. Reject an ambiguous or unverifiable layout without changing the database.
   Never assume Rust byte ordering happens to match `localeCompare()`.

For each graph fixture:

- the supported legacy compiler and recorded legacy manifest report the same
  parameter count and node offsets;
- the canonical compiler has independently fixed expected offsets;
- legacy and canonical layout versions/keys are distinguished explicitly;
- ASCII case, numeric-looking, punctuation, non-ASCII, and locale-sensitive
  node IDs have Windows/Node, Debian/Node, migration, and Rust fixtures;
- distinct canonically equivalent Unicode spellings remain separate, totally
  ordered IDs;
- legacy implicit Concat orders are made explicit and preserve downstream
  feature/weight meaning;
- deterministic weights produce outputs within explicit numeric tolerance;
- recurrent state transitions match for multi-step sequences;
- Split/Concat/custom output mapping is covered;
- different genomes in the same population produce their own outputs;
- shuffled active-snake order does not migrate recurrent state.

### Production evaluation

One internal Rust neural pass receives stable due brain handles (dense evolved
slots and eligible non-population brains) and:

1. builds or reads their sensor rows;
2. looks up each handle’s own weight block and compiled-graph layout;
3. executes the complete compiled graph;
4. updates that handle’s own recurrent state;
5. writes the two outputs;
6. returns to the coordinator only after all due handles are complete.

Resurrected snakes use their supplied genome and fresh recurrent state. An
external snake in lease/grace state performs no neural evaluation; if the
owner selects post-grace takeover, that one-time transition creates or enables
its explicitly owned brain and cannot borrow an evolved population slot.

The existing SIMD arithmetic may be reused internally after safety/parity
tests. It is no longer the production language boundary.

## Movement, food, growth, and death

The port maps current behavior before optimizing:

- turn rate and size penalty;
- base and boost speed interpolation;
- boost fuel threshold, point cost, size factor, shrink, and pellet trail;
- boundary death behavior;
- body following and spacing;
- food radius and value;
- growth/shrink toward target length;
- score, food count, age, and kill accounting;
- normal and God Mode death side effects;
- corpse pellet count/value/jitter/color;
- baseline death notification;
- skin and rendering metadata.

Known allocation patterns such as a fresh pellet candidate array for every
snake/substep are not copied. Their results are reproduced with reusable
scratch and two-phase claims.

## Collision and spawn rules

### Broad phase

- Size the index from actual active segments/cells with checked arithmetic.
- Insert every cell touched by a segment’s swept bounds from its previous to
  proposed endpoints, expanded by collision radius.
- Never silently truncate.
- Reuse allocations between substeps and generations.
- Clear all references/handles on reuse.

### Narrow phase

- Use relative-motion tests between each swept head and moving body segment so
  either object crossing during the substep is detected. A final-position-only
  body index is insufficient.
- Detect two heads crossing between endpoints even if they do not overlap at
  the final positions.
- Compute continuous head-versus-segment capsule distance using both snake
  radii and hit scale, or prove with a tested displacement bound that a simpler
  discrete test is equivalent for every allowed setting.
- Preserve current no-self-collision behavior unless the owner requests a game
  rule change.
- Detect from immutable state.
- Deduplicate candidate segments.

### Commit

- Record all deaths before applying any.
- Apply in stable ID order.
- Make outcome independent of snake array order and worker count.
- For head-to-body death, provisionally award the kill to that body’s owner.
  Simultaneous head-to-head death awards neither participant a kill unless the
  Stage 2 current-behavior/examples show a different intended rule and that
  change is reviewed before finalization.
- Ensure a snake killed in the same collision snapshot still counts as an
  obstacle for that snapshot.

### Head-head and contested-pellet rules

Two heads colliding in the same immutable collision snapshot kill both snakes.
For food, the nearest eligible head wins and exact distance ties use stable
snake ID. Stage 2 shows the current behavior and concrete collision/credit
examples before the provisional kill-credit detail above is finalized.

### Spawn placement

- Generate candidates from the appropriate deterministic RNG.
- Check the complete initial body, not only the head.
- Require configured clearance from walls and existing bodies/heads.
- Use bounded rejection sampling.
- If random attempts fail, use a deterministic spatial fallback.
- If the requested population physically cannot fit, reject the reset with a
  clear message instead of starting a biased collision pile or silently
  reducing the configured population.
- Prove that reversing population storage order does not change which spawn
  locations belong to stable slots for the same versioned algorithm.

## Evolution and generation rules

Rust ports and tests:

- population-slot identity;
- fitness components and normalization;
- best/average/min history;
- species and network statistics;
- stable fitness sorting and tie behavior;
- elite count;
- tournament selection;
- recurrent block crossover;
- mutation rates, standard deviations, and gate biases;
- Hall-of-Fame selection and metadata;
- early generation end;
- run-start and generation-boundary checkpoint ordering;
- reset with same seed/new run ID;
- New Run with entropy seed/new run ID;
- import replacement behavior;
- resurrection outside the dense evolving population.

Evolution initially uses the current xorshift32 and Box-Muller algorithms and
the same serial draw order where behavior is intended. Corrected external
controller creation gets its own stream so connection bookkeeping does not
advance breeding or world streams.

Physical player/RL interaction may intentionally change a live round and
therefore its fitness outcomes. Merely opening, dropping, or reclaiming a
connection must not silently shift unrelated random continuation.

## Display frames and browser load

### Compatibility stage

Rust first packs serializer v1 exactly:

- the seven-float header and its existing field positions;
- alive snakes in defined world order;
- eight scalar snake fields;
- body point pairs;
- pellet count and five pellet fields;
- existing pellet type and color encoding.

The first four v1 header fields remain authoritative generation/count/radius
data. The final camera X, camera Y, and zoom fields are explicitly
presentation-only. The browser owns and smooths those values and may send its
latest view descriptor to Node for Rust to echo into a per-connection v1 frame;
clients that send none receive documented neutral defaults. They are not
generated by an observer RNG, do not enter checkpoints, and cannot affect
simulation. Byte-parity fixtures provide an explicit view descriptor and
expect an exact echo; parity does not preserve the obsolete randomly selected
server camera behavior.

The current browser parser, renderer, selection, and tests must accept the
Rust-produced buffer without rebuilding it in TypeScript.

The Rust frame publisher or thin routing layer also retains the latest packed
frame byte length as small welcome metadata. Refreshing welcome state reads
that value; Node never serializes or reconstructs the complete world merely to
calculate `frameByteLength`.

### Optional measured display stage after the first cutover

Serializer v1 is inherently a full-world format. A negotiated later serializer
adds:

- per-connection view/follow information;
- viewport plus safety-margin culling;
- full nearby/player body detail;
- distance/overview body-point decimation;
- pellet culling or aggregation outside useful view;
- explicit total/alive counts separate from displayed entity count;
- sequence/tick metadata so an older frame cannot replace a newer one;
- one latest frame per UI, never a frame queue.

This work begins after the first usable Rust cutover unless v1 measurements
show that the supported workload cannot meet player-control latency. The
browser and server negotiate any later format in `welcome`; v1 remains
available for compatibility. God Mode selection continues to have enough
ID/head information in the displayed region.

### Browser acceptance

Moving the game to Rust is not considered successful if the laptop still
becomes unusable parsing/rendering oversized frames. The stress matrix records:

- bytes per frame and per second;
- server frame-pack time;
- WebSocket buffered bytes;
- browser parse time;
- browser render time;
- displayed frame rate;
- player input-to-visible-response latency.

## Stats, visualization, and presentation hints

- Rust calculates alive population, alive total, baseline totals, generation
  time, and committed tick.
- Existing fitness/history/Hall-of-Fame payload meanings are either preserved
  or corrected under a documented protocol change.
- Neural visualization captures only explicitly requested/focused activations.
- Visualization work is disabled when no subscriber requests it.
- An optional focus suggestion is an uncheckpointed presentation ID selected
  explicitly by a UI or by a stable non-random rule. There is no active
  observer/camera RNG in the Rust game.
- Legacy checkpoint observer-RNG data may be read and preserved as inert
  compatibility metadata, but it is never advanced or used by the new engine
  and is deprecated in the next checkpoint version.
- Multiple UI view subscriptions must not change simulation state.
- A slow spectator cannot affect player/RL control queues.

## Persistence and API compatibility

Persistence is part of the runtime design, not an HTTP afterthought. Node
continues to own SQLite transactions and ordinary HTTP file transfer. Rust
owns authoritative checkpoint construction, validation, archive encoding and
decoding, and staged replacement state. Neither side expands population
weights into decimal JSON.

### Persistence execution isolation

The current `better-sqlite3` API is synchronous. Running legacy
multi-hundred-MiB reads/conversion, WAL checkpoints, or `VACUUM` on the main
Node event loop would recreate player/RL starvation even after the game moves
to Rust.
Therefore one dedicated Node persistence worker thread owns the SQLite
connection and performs every blocking statement/chunk iteration. The main
Node thread sends correlated small requests, streams ready files with normal
asynchronous file APIs, and remains available for WebSocket/control/health
traffic. Rust archive work runs on its background engine/archive threads.

This is a storage worker, not another inference pool or authoritative game.
It never sees a `World` and cannot advance simulation. Startup, shutdown,
faults, transaction results, busy state, and operation IDs have one bounded
message contract. Stage gates measure main-event-loop delay and player/RL
latency during checkpoint, prune, export composition/download, import,
legacy conversion, WAL maintenance, and explicit compaction.

The persistence worker is priority scheduled between bounded SQLite
statements/chunks:

1. required run-start/generation/current-pointer barriers and startup recovery;
2. small current metadata/compatibility lookups;
3. user export history/Hall-of-Fame inventory assembly;
4. pruning metadata, WAL maintenance, and legacy compaction work.

An import's final state-replacement transaction is explicitly exclusive and
pauses the simulation with visible status; upload and Rust validation happen
before that pause. Export reads immutable managed checkpoint/content files and
a bounded SQLite history/reference inventory under an export reference, so it
does not hold one long read transaction or pin the WAL. Legacy conversion and
maintenance check for higher-priority work after at most one bounded chunk or
statement. A statement whose measured worst case cannot meet that yield bound
is redesigned or moved to explicit offline maintenance. A P2 export
overlapping a generation boundary must still meet the checkpoint and
Node-control latency gates.

### Bounded bulk handoff between the persistence worker and Rust

Population bytes never travel through Node worker structured cloning, an
unbounded message queue, or the main Node isolate:

- Rust writes checkpoint/import candidates directly as controlled managed
  files, closes/fsyncs/renames them, then sends only operation ID, relative
  managed path, byte counts and logical root to the persistence worker;
- the persistence worker commits only the small metadata/current pointer; it
  never copies the checkpoint bytes into SQLite;
- for v3 startup/resume, the worker returns only the validated controlled path
  and expected metadata/root, and Rust opens the immutable file directly;
- Export reads that managed file plus the worker's bounded history/
  Hall-of-Fame reference inventory and composes the standalone `.partial`
  archive without routing population bytes through Node;
- only legacy-database conversion may require the worker to write selected
  BLOB/TEXT slices to a controlled `source.partial` spool in bounded pieces;
- each path is an operation-UUID child of the dedicated temp directory, never
  user supplied, and ownership passes only after close/fsync/ack;
- cancellation, worker failure, and restart close handles before cleanup; an
  active producer/consumer holds an explicit reference so scavenging cannot
  remove its file.

The operation state machine permits at most two large spools at once:
legacy-source/upload plus candidate/output. Normal v3 export needs only its
output spool because its checkpoint source is already immutable. Import
deletes its upload only after the candidate is durably committed or rejected;
no transition may hold an unnecessary third population-sized copy. Worst-case
files are included in the configurable temp quota and disk formula. Tests
trace worker message sizes and queue depth, proving no population-sized
`ArrayBuffer`, string, or structured-clone backlog exists.

### Current growth inventory

The current format is bounded per genome but unbounded over time:

- every automatic generation checkpoint duplicates the complete population as
  raw little-endian Float32 child rows;
- the default checkpoint interval is every generation;
- there is no retention or pruning path;
- each generation's best genome is repeated as decimal JSON in checkpoint
  metadata and in `hof_entries`;
- SQLite free-page and WAL maintenance are not controlled by the application;
- the current JSON export drops some authoritative metadata despite expanding
  every weight into text.

The current schema and write cadence prove that weight storage accumulates.
A prior planning measurement reported two default 55-genome run-start
snapshots containing 5,921,520 weight bytes in a 6,041,600-byte SQLite file,
with about two percent non-weight overhead. That database, extraction command,
raw output, SQLite/WAL state, and environment are not retained in the audited
tree, so Draft 4 treats the figures as provisional until Stage 2 reproduces
them on a named fixture.

The following planning arithmetic uses current graph formulas and the prior
provisional observation of about 20.03 decimal-JSON text bytes per Float32.
Raw packed weights are exactly four bytes each; the “current growth” and
eight-hour columns additionally estimate the two current decimal copies of the
best genome and exclude small B-tree/WAL overhead. Stage 2 replaces the
provisional JSON factor and resulting columns with artifact-backed results.

| Workload | Weights/genome | Raw population/checkpoint | Approx. current growth/generation | Eight hours at one 60-second generation |
|---|---:|---:|---:|---:|
| P0: 55, default brain | 13,458 | 2.82 MiB | 3.34 MiB | 1.57 GiB |
| P1: 300, default brain | 13,458 | 15.40 MiB | 15.92 MiB | 7.46 GiB |
| P2: 55, large brain | 402,914 | 84.53 MiB | 99.93 MiB | 46.84 GiB |
| P3: 300, large brain | 402,914 | 461.10 MiB | 476.49 MiB | 223.35 GiB |

At the current default 240-second generation, divide the eight-hour column by
four. Early deaths, faster requested simulation, and shorter configured rounds
increase generations per wall hour. These estimates explain how an overnight
run can reach tens of gigabytes without requiring a single abnormally large
write.

### Recommended archive: one simple USTAR file with adaptive payload encoding

Draft 4 recommends one ordinary USTAR container for each new save:

```text
slither-neuroevo-<run>-gen-<generation>-v1.slither-save
```

The HTTP type is `application/vnd.slither-neuroevo.save`. The archive is one
file. Large numeric entries individually use whichever of these versioned,
bit-exact encodings is smaller for that actual payload:

- `raw-f32le-v1`: packed four-byte little-endian Float32 values;
- `f32le-shuffle4-zstd-v1`: the same bytes grouped by byte position and then
  compressed as one or more bounded Zstandard frames.

The encoder streams a shuffled-Zstandard candidate to a bounded temporary
file, compares its actual stored bytes with the raw packed length, and keeps
the compressed candidate only when it is smaller. Otherwise it stores the raw
packed entry. It never keeps both candidates in memory. Compact non-Float32
records may similarly choose versioned raw or Zstandard encoding when measured
size justifies it.

This adaptive rule replaces draft 2's claim that every population must beat a
fixed compression percentage. High-entropy evolved weights may legitimately
compress little or not at all. Packed binary plus retention supplies the
guaranteed size bound; compression supplies an additional measured saving when
the data permits it. A numeric entry therefore never expands beyond its raw
packed bytes except for small archive/header metadata.

Brief practical comparison:

| Candidate | Relevant trade-off | Draft 4 decision |
|---|---|---|
| USTAR with adaptive raw or shuffled-Zstandard entries | Mature Rust `tar`/`zstd` support, direct per-payload size choice, ordinary tar listing, no double compression | Selected |
| One outer `.tar.zst` frame | Simple filename, but cannot independently leave incompressible numeric payloads raw and makes per-entry diagnosis/selection awkward | Rejected for new v1 |
| tar + gzip or ZIP/Deflate | Broad tooling, but slower/weaker compression for this workload and no benefit over the selected simple container | Legacy readers only |
| Custom private container | Could remove tar headers, but adds unnecessary format code and diagnostic tooling | Rejected |

Node does not compress, decompress, or inspect archive bodies. Rust performs
the codec work against controlled files; Node only streams the finished opaque
file or spools an upload. Compression level 3 is the initial candidate, then
Stage 2 measures raw, shuffled-Zstandard, encoding time, and restore time for
fresh and evolved P0/P2/P3 populations on the Ryzen 7 2700.

A prior planning measurement reported that plain level-3 Zstandard reduced
2,960,760 raw P0 run-start bytes to 2,733,479 bytes (7.68 percent) and
byte-shuffle plus Zstandard reduced them to 2,527,050 bytes (14.65 percent).
The population fixture, script, raw output, codec build/settings, and
environment are not retained in the audited tree. These are provisional
observations to reproduce in Stage 2, not independently verified facts or a
required ratio for any future genome. Failure to reproduce them updates the
codec and disk assumptions rather than preserving the old numbers silently.

### Archive v1 contents

The owner has selected one self-contained exact generation-boundary experiment
as the ordinary export. Mid-round saves and a separate population-only export
are outside the current scope.

The USTAR file contains these logical roles:

1. `checkpoint.json` — small structured authoritative state;
2. `history.bin` or `history.bin.zst` — every compact generation-summary
   record;
3. `population/index.bin` — slot, architecture, fitness, weight offset/count,
   and recurrent-state offset/count;
4. exactly one of `population/weights.f32le` or
   `population/weights.f32le.shuf4.zst`;
5. exactly one raw or shuffled-Zstandard recurrent-state entry;
6. `hof/index.bin` — compact run-scoped Hall-of-Fame metadata and references;
7. exactly one raw or shuffled-Zstandard Hall-of-Fame weight entry when unique
   weights are required;
8. `manifest.json` — the final small entry, listing the preceding logical
   roles, selected encodings, logical lengths/counts, logical SHA-256 digests,
   versions, and one logical checkpoint root.

Putting the manifest last lets the writer calculate lengths and hashes while
streaming each entry once. A truncated archive without the final manifest is
invalid. Import never makes staged data authoritative before the complete
manifest and every declared role have been checked.

Small JSON may contain strings, booleans, versions, graph definitions,
settings, and other genuinely small records. It never contains a
population-sized numeric array.

`f32le-shuffle4-zstd-v1` is lossless. Each independent block contains a
Float32 count followed by four byte planes, with a bounded decoded block size.
The Zstandard frame must declare its decoded content size and decoder window
before allocation. Decoding restores the original four-byte values bit for bit
using one reusable bounded scratch buffer. Index offsets count logical
Float32 values, not archive or compressed-byte positions.

`manifest.json` contains:

- magic `slither-neuroevo-save`, archive version, and archive kind;
- engine snapshot, sensor, RNG, graph-layout, serializer, and history versions;
- every required logical role, actual tar entry name, selected encoding,
  stored length, decoded length/count, record size, and logical SHA-256;
- total stored and decoded bytes used for admission;
- population, weight, recurrent-state, history, and Hall-of-Fame counts;
- graph architecture key and ordered parameter-layout digest;
- run identity, source provenance, exact-versus-compatible continuation status,
  history coverage, and run-scoped Hall-of-Fame coverage;
- one logical root digest over the archive version and ordered
  `(role, encoding-independent logical length, logical SHA-256)` list.

`checkpoint.json` contains the run ID, seed, RNG streams, allocator state,
generation, completed fixed-step count, generation-boundary kind,
authoritative settings and updates, graph definition/layout, config identity,
best-fitness state, build/target compatibility identity, compact-history
extent, and Hall-of-Fame policy/version. Exact archives contain complete
run-scoped Hall-of-Fame and compact-history data. Legacy conversions record
their actual provenance and completeness and never invent missing state.

`history.bin` keeps the existing eight values per generation as a versioned
56-byte record. Population and Hall-of-Fame indexes retain explicit magic,
version, record size, counts, checked logical offsets, stable slots/IDs,
architecture references, fitness fields, and weight-source references. All
length arithmetic is checked before allocation; binary integers and IEEE-754
values are little-endian, and authoritative floats must be finite.

### Minimum archive integrity layers

Each integrity mechanism has one distinct job:

| Mechanism | Needed? | Reason |
|---|---|---|
| Safe USTAR parsing and header checksum | Yes | Finds damaged container headers and rejects duplicate, missing, unsafe-path, link/device, or unknown entry types. |
| One logical SHA-256 per role | Yes | Identifies which logical record is corrupt and verifies raw or decoded compressed bytes with the same rule. For numeric entries this is also the decoded-array hash; there is no second hash. |
| One logical root digest | Yes | Gives SQLite/current-pointer/import-conflict/content-reference code one compact identity that binds the version and complete ordered logical record set. It does not depend on tar timestamps or compression choices. |
| Zstandard frame content checksum | Measurement-gated, off in the provisional simplest path | The logical SHA-256 detects corrupted decoded content. Stage 2 may enable the cheap frame checksum for compressed entries if its measured cost is negligible and its earlier codec-level diagnosis is useful; it does not replace the logical hash. |
| Encoded-byte hash in addition to logical hash | No | Decode failure identifies malformed compressed data; successful decode is covered by the logical hash. |
| Footer digest table | No | The final manifest already carries the one required digest table. |
| Separate manifest hash | No | The logical root binds the manifest's ordered role declarations and is checked against the reconstructed logical records. |
| Canonical uid/gid/mtime/mode bytes | No | Archive-byte reproducibility is not a product requirement. Writers use ordinary regular-file headers and safe fixed names; logical identity ignores presentation metadata. |
| Mandatory full post-write decode | Not for every automatic checkpoint | Write-time counts/hashes plus flush, fsync, final file-length/container completion checks catch ordinary writer failures. Import/startup/restore performs full validation before loading authority. Stage 2 may justify full decode for manual exports, pinned checkpoints, or periodic milestones, but automatic generation checkpoints are not decoded twice by default. |

No additional checksum, canonicalization layer or container feature is added
without a distinct failure it prevents. Archive work is complete when safe
bounded import, useful corruption diagnosis, logical content identity and the
round-trip tests pass; building a broader private storage standard is outside
scope.

#### Write-path validation trade-off

The provisional simplest policy is a single write pass for ordinary automatic
generation checkpoints. While generating the managed checkpoint or export,
the writer calculates logical hashes and counts from the source bytes, finishes
the codec/container, flushes and fsyncs the file, checks completion and final
length, then publishes it atomically. Eligibility does not necessarily wait
for a complete decode of the newly written compressed file.

That avoids approximately doubling codec and read I/O on every generation
boundary, but it deliberately trusts source hashing plus successful codec and
storage completion until startup, import, restore, or another validation
operation decodes the file. A latent codec or storage defect can therefore be
discovered during restore rather than creation; the retained previous valid
checkpoint and the owner-selected recovery-branch rule are the protection.

Stage 2 measures four bounded choices on named P0/P2 fixtures: the single-pass
policy; enabling the Zstandard frame checksum for compressed entries; a
lightweight USTAR/entry/completion scan without decoding every numeric payload;
and a complete post-write decode limited to manual exports, pinned checkpoints,
or periodic milestone checkpoints. The least complicated option that provides
useful diagnosis without violating checkpoint-latency targets is retained and
recorded. Draft 4 does not restore encoded-byte hashes, duplicate decoded-array
hashes, footer tables, canonical tar metadata, or a mandatory second full
decode for every automatic checkpoint unless measurement identifies a distinct
failure that the minimum layers do not cover.

The retained development-machine comparison was produced from clean source
commit `ac905db49bbb912bf49cf3a91b36934c72932229`. The P0 artifact has SHA-256
`efa57db87552c61452ebb48240d49c562c50b33ecb254a4d4a5a2cfd40bb7e96`; the P2
artifact has SHA-256
`59bbd3013ea85bb65b0894b24633305f2293b115ede99f9047ad1d3f2055caed`.
On the Windows Ryzen 7 5800X, P0 single-pass publication had 33.613 ms p95,
lightweight scanning added 2.079 ms p95, and full decode produced a 50.803 ms
publication-barrier p95. P2 single-pass publication had 587.623 ms p95,
lightweight scanning added 9.581 ms p95, and full decode produced a 965.716 ms
publication-barrier p95. The strict fault matrix proved that a structural scan
does not verify compressed payload content, and a Zstandard checksum is not
checked until decoding occurs.

The minimum Stage 3 policy selected from that evidence is one-pass logical
hash/count generation, codec/container completion, file flush and fsync, final
length check, atomic rename, and Debian parent-directory fsync for ordinary
automatic checkpoints. It leaves the optional Zstandard frame checksum off and
does not add a payload-blind second scan to automatic publication. Strict
validation remains mandatory when startup/import/restore consumes an archive;
manual exports and pinned checkpoints receive a full post-write decode when
implemented. Periodic-milestone full decode remains measurement-gated. The
retained prior checkpoint and recovery-branch rule protect against a latent
fault discovered during restore. These are disposable Node measurements, not
the production Rust codec or target-VM proof: P2's development-machine result,
Windows directory-fsync `EPERM`, and process RSS still require Rust/Debian
measurement before any final latency, durability or memory gate can pass.

The importer rejects duplicate logical roles, absolute/backslash/parent paths,
links, devices, sparse/extension records not supported by v1, impossible
counts, overlapping/out-of-range indexes, unsorted population slots,
unsupported encodings or versions, decoder windows or declared sizes over
limits, missing final manifest, logical-hash/root mismatch, incompatible graph
ordering, truncation, and trailing non-tar data. It reports the first failed
logical role and leaves the original file and current run untouched. The
program does not attempt best-effort partial recovery.

### Direct browser download

The normal export path is:

1. The user clicks Export.
2. The browser immediately activates an ordinary link or navigation to one
   `GET /api/export/latest` download endpoint. It may show a small “download
   requested/preparing” label, but it does not fetch the response into
   JavaScript.
3. That one request binds an export operation and temporary reference to one
   exact durable checkpoint/root digest. The server keeps the request open
   while it prepares the file; a concurrent export request receives a small
   plain `409 busy` response.
4. Rust writes a unique same-directory `.partial` archive with bounded
   buffers while checking counts and logical hashes. It finishes the tar,
   flushes/fsyncs it, checks the final file length, atomically renames it to
   `.ready`, and fsyncs the parent directory where required. It does not
   perform a second full decode merely for ceremony.
5. Only after archive creation succeeds, Node answers the original request by
   streaming that exact ready file with `Content-Type`, `Content-Length`,
   and `Content-Disposition: attachment` using a safe versioned filename.
6. The browser's normal download system writes the bytes to the selected
   destination. Browser JavaScript does not inspect the archive, call
   `response.json()`/`.text()`/`.arrayBuffer()`, read `Response.body`, use a
   stream reader, call `JSON.parse()`/`JSON.stringify()`, call
   `Array.from(weights)`, or create a population `Blob`.
7. If preparation fails before response headers, the endpoint returns one
   small plain error response. If the user closes/cancels the request, the
   server cancels or finishes bounded cleanup without retaining a hidden
   downloadable copy.
8. The server deletes the temporary file after successful response close,
   preparation failure, response failure, or cancellation.

The download request acquires a temporary export reference to one exact
durable checkpoint and records its root digest in the operation. A later
generation, checkpoint, Reset, New Run, or pruning pass cannot change or
delete the bytes that operation will encode. The response filename, manifest,
and downloaded file must all name that same checkpoint. The reference is
released only after response/cleanup is terminal. A failed or cancelled
transfer requires the user to click Export again; v1 does not retain a hidden
retry copy. Tests exercise two concurrent export requests, cancellation before
and after headers, successful close, preparation failure, and a fresh request
after failure.

This draft chooses a bounded temporary file over live response generation
because it lets the server detect encoding/checksum failure before committing
HTTP success, supplies a stable length, avoids a half-valid archive, and keeps
the implementation simple. It does use temporary disk roughly equal to the
finished archive, which is included in admission and free-space checks.

The page may report only that the download was requested. A preparation error
is the endpoint's small plain response and is also logged server-side; the page
does not claim that the user saved the attachment because browser JavaScript
cannot reliably know that. The archive is obtained by the original ordinary
attachment navigation and is never fetched into JavaScript.

Tiny browser-only preferences are stored through their normal small settings
route before export or remain local presentation preferences. They never
justify sending the population to browser JavaScript.

### Direct binary upload

The normal import path is:

1. The browser presents a file picker.
2. The selected `File` itself is the raw request body of a dedicated binary
   upload route. V1 uses `XMLHttpRequest.send(file)` so the page can show
   ordinary upload-byte progress without reading the file; an implementation
   may use `fetch` only if it can supply the required small progress/error
   state. V1 does not add multipart parsing.
3. Browser code never calls `FileReader`, `.text()`, `.arrayBuffer()`,
   `JSON.parse()`, or population `JSON.stringify()`.
4. Node checks `Content-Length` when present, counts actual received archive
   bytes, and spools them to a unique `.partial` file. It does not concatenate
   the body in memory.
5. Rust validates safe USTAR structure, per-entry encoding and declared sizes,
   the final manifest, logical hashes/root, graph ordering, and every record
   while streaming from the spool.
6. Rust constructs a candidate replacement state that is not authoritative.
   The existing world remains current while this candidate is built.
7. Rust writes/publishes the candidate managed checkpoint file; the persistence
   worker records only its inactive/staged metadata and references.
8. At one safe engine boundary, the already-validated candidate and its
   correlated durable record become current. A post-prepare engine failure
   faults the process and resumes the committed record on restart; it never
   falls back to a partly modified old/new mixture.
9. Only after success does the browser apply the small returned settings/run
   state. Failure leaves the current engine, database current pointer,
   settings, graph, history, and Hall of Fame unchanged.
10. The spool is deleted after success, validation failure, cancelled upload,
     or server error.

Exact-identity import never silently overwrites or merges existing rows. Before
commit, every archive `(run_id, generation, record_kind)` key is compared with
the destination:

- an existing row with the same version/root/content digest is reused
  idempotently;
- the same key with a different root/content digest rejects the entire import
  as an identity conflict;
- an exact archive whose run ID has no later destination history may restore
  that saved experiment identity;
- if later generations already exist, normal import cannot move the same run
  backward and continue into colliding future keys. The owner-selected
  “Resume as branch” operation creates a new run ID with source
  run/generation/root provenance, preserves the original suffix, and continues
  from the archived RNG/population state without calling the identity exact;
- a clone, merge, or destructive replacement—if wanted—is a separately named
  operation with separately reviewed identity/history/Hall-of-Fame rules.

Tests repeat an identical import, import a same-key/different-root fixture, and
attempt an older same-run boundary with existing future history. The branch
test advances the next generation without key collisions; none may silently
replace, combine, or delete data.

The generic 50 MiB JSON-body limit does not apply. The following are
provisional, configurable Stage 2 measurement starting values for the 16 GB
VM, not permanent product limits:

- 4 GiB maximum received archive;
- 8 GiB maximum declared decompressed entry bytes;
- 16 MiB maximum manifest plus structured metadata;
- 10,000 population slots and 2,000,000 weights per genome, preserving the
  existing per-record ceilings;
- 128 MiB maximum Zstandard decoder window;
- a calculated total-process peak no greater than 12 GiB, including the
  current engine, staged engine, SQLite, Node, archive buffers, and expected
  page cache.

Content length over the current configured archive limit is rejected before
upload with HTTP 413. A false/missing declared size for a compressed entry,
unsafe decoder window, aggregate weight count, or projected process peak is
rejected before constructing replacement state. Errors state received archive
bytes, declared decoded bytes, record counts, calculated state size, free disk,
and the applicable configured limit in plain language. A normal export performs
the same admission calculation and cannot emit a file that its normal import
profile would reject. Stage 2 derives the final defaults from actual state,
record counts, free disk, and measured peak memory.

Temporary files use a dedicated data-directory subtree and an operation UUID,
never a user-selected pathname. Clean exits remove them. Startup removes only
matching unreferenced `.partial`/`.ready` files older than 24 hours; it never
scans or deletes browser download locations or configured export directories.

The proposed transport liveness default is a 60-second **no-progress** timeout
for an upload or an attachment response after body transfer begins, reset
whenever bytes move. There is no short total-duration cap: a large slow
transfer may continue as long as it progresses. Archive preparation before
response headers is governed by the separate server-work heartbeat/watchdog,
not falsely treated as a stalled download. A connected transfer that moves no
bytes past the idle limit is aborted, its file/reference is released, and
normal cleanup applies. Encoding, validation, database chunking, and Rust
construction expose separate per-chunk heartbeats rather than pretending
socket activity proves server work is healthy.

User-requested export preparation and import are process-wide serialized
initially. Required automatic checkpoints remain higher priority and may
interleave with export-source assembly at the bounded yield points above; the
exclusive import commit pauses stepping. The temporary subtree begins with a
provisional/configurable 9 GiB aggregate quota so one maximum 4 GiB
upload/source and one maximum 4 GiB candidate/output spool can coexist during
an atomic operation. Stage 2 replaces these starting values from measured
state, free disk and VM memory. A ready export still being downloaded counts
against the quota. A second user archive operation receives a small
busy/required-space error instead of starting another large spool.

Before a checkpoint, import, export, or pin starts, admission estimates:

```text
existing temp files
+ every source/upload/candidate/output spool still required
+ the final managed checkpoint/content files not yet pruned
+ the measured small-metadata SQLite/WAL transaction allowance
+ 1 GiB operating reserve
```

Free disk must exceed that sum. Pruning occurs only after the new checkpoint
commits, so bytes expected from later pruning do not count as available.
Failure leaves the old durable/current boundary intact and reports every term;
it never deletes first and hopes the replacement fits. Stage 2/7 measurements
may justify bounded concurrency later, but concurrent jobs never bypass the
combined disk and 12 GiB process-memory admission calculations.

### Checkpoint storage comparison

Draft 2 assumed that compressed checkpoint streams would be copied into
`snapshot_archive_chunks`. Draft 4 does not inherit that assumption. The
written architectural comparison remains useful, but Stage 2 does not build
two equivalent persistence systems:

| Concern | Compressed stream in SQLite chunk rows | Managed immutable checkpoint file plus SQLite metadata |
|---|---|---|
| Implementation complexity | Chunk schema, sequencing, large transaction loop, reassembly, WAL/reader coordination and reference-aware BLOB cleanup | Ordinary temp-file write, atomic rename, small metadata transaction and controlled directory cleanup |
| Crash consistency | One database commit can contain payload and pointer, but it is a large commit with a large WAL | Final file is fsynced/renamed before the small pointer transaction; a pre-commit crash leaves a harmless orphan, never a pointer to a partial file |
| Checkpoint latency | Copies every compressed byte through SQLite and its WAL before commit | Writes each byte once to its final filesystem form, then commits small metadata |
| Restore latency | Iterates/reassembles many rows before Rust can stream the record | Rust opens one immutable file and streams it directly |
| Export cost | Reassembles/decompresses database chunks, then combines referenced history/Hall-of-Fame data | Uses the checkpoint file directly as the authoritative source and adds compact history/Hall-of-Fame records to the standalone export |
| Backup | One nominal database artifact, but a correct hot backup must still include the WAL and may be very large | Backup must include SQLite plus the managed directory; immutability and a database-generated file inventory make this explicit |
| Pruning | Deletes rows and later performs reference cleanup | Removes metadata/references first, then unlinks unreferenced immutable files |
| Disk reclamation | Deleted pages are reusable but the database file may not shrink without vacuum/compaction | Unlinking a file returns filesystem space immediately |
| WAL growth | Population-sized transactions can create population-sized WAL growth | WAL contains only metadata, history and reference changes |
| Database compaction | Large deleted BLOB regions can require incremental or full vacuum | No large checkpoint BLOB pages exist; normal small-database maintenance remains |
| Failure recovery | SQLite handles payload atomicity, but large commit/reply windows and corruption affect the main database | Startup detects orphan files and missing/digest-mismatched referenced files separately; prior valid files remain independently readable |

The managed-file design is selected for Draft 4 because it has the smaller
implementation and operational surface and no concrete correctness requirement
currently requires population-sized SQLite BLOB transactions. Stage 2 may run
one narrow disposable SQLite BLOB/chunk throughput experiment with
representative compressed byte volumes if empirical confirmation is useful.
That experiment may measure only bytes written, transaction latency, WAL
amplification, read-back throughput, deletion/page reuse, and main-event-loop
impact when synchronous work is placed incorrectly.

It must not create a competing checkpoint schema, complete chunk writer/reader,
backup path, pruning/garbage collection, crash-recovery state machine, export
path, or equivalent P0/P2/P3/overnight end-to-end persistence infrastructure.
All complete checkpoint publication, restore, backup, pruning, recovery,
interruption, compaction interaction, and overnight-volume tests exercise the
selected managed-file design. Switching new writes to SQLite chunks later
requires a concrete correctness/deployment problem or a measured failure of
managed files, followed by review. “Implement both fully to compare them” is
not sufficient. The selected path does not create
`snapshot_archive_chunks`.

### Selected managed checkpoint directory

New checkpoint-v3 payloads live under one server-controlled checkpoint
directory on the same filesystem used for atomic publication. SQLite stores
only:

- run/generation/boundary metadata, logical root and relative managed filename;
- encoded/decoded byte counts, selected retention class, pin state and current
  pointer;
- compact generation history;
- graph/config version records;
- run-scoped Hall-of-Fame indexes and references;
- operation/provenance/status records required for recovery and cleanup.

Paths are generated from validated content digests and fixed directory
components, never supplied by an HTTP client or read unchecked from an
archive. Checkpoint files are immutable after publication. Packed unique
Hall-of-Fame genomes that are not retained inside a referenced checkpoint use
the same controlled content directory as small independently hashed binary
objects; SQLite stores their indexes and references, not decimal weights.

An internal checkpoint-v3 file uses the same raw-or-shuffled-Zstandard numeric
encodings, checked record layouts and logical-root rule as archive v1, but it
contains only the exact generation-boundary engine/checkpoint state needed to
resume. It is not presented as a standalone export because complete compact
history and the run-scoped Hall of Fame remain referenced separately.
Standalone Export streams the selected checkpoint file plus those referenced
small/binary records into one self-contained `.slither-save` file without
materializing population data in Node.

Checkpoint publication order is:

1. Rust streams a unique `checkpoint-v3.partial` beside the managed directory
   destination, calculating counts, logical hashes/root, and adaptive encoding
   choices with bounded memory.
2. It finishes the container, flushes/fsyncs it, checks final length, renames it
   atomically to its digest-derived immutable filename, and fsyncs the
   directory where required. Under the provisional single-pass policy, this
   makes the checkpoint eligible without decoding every numeric entry a second
   time; its metadata records the encoding and write-validation policy used.
3. The persistence worker begins one small `synchronous=FULL` transaction and
   inserts the checkpoint metadata, one run-scoped history record, pending
   Hall-of-Fame/reference changes, and the new current pointer.
4. The transaction commits and returns its matching operation/transition epoch
   to Rust.
5. Rust accepts only that matching result, swaps the staged world at the safe
   boundary, and publishes success/lifecycle events.
6. Retention runs separately after the new checkpoint is current.

A crash before file rename leaves only a recognized partial file. A crash
after rename but before SQLite commit leaves an unreferenced immutable orphan.
A crash after commit always finds the referenced final file because the file
was durable first. Startup removes partial/orphan files only after checking
SQLite references and the documented grace period. A database reference to a
missing or digest-invalid file is corruption and follows the owner-selected
latest-recovery rule; it is never silently ignored.

Pruning first commits removal of unpinned metadata/references, then unlinks
only files with no remaining checkpoint, pin, Hall-of-Fame, export, import or
backup reference. A crash between those actions leaves an orphan for later
cleanup, not a dangling live pointer. A failed new checkpoint never deletes an
older one.

Backups are explicit because the durable state is now a set:

- the simplest cold backup stops the service and copies SQLite plus the entire
  managed checkpoint/content directory;
- a supported hot backup acquires a short database/file-inventory reference,
  uses SQLite's backup API for a consistent database image, copies every
  immutable file named by that inventory, and validates their roots before
  releasing the reference;
- a self-contained Export archive remains the simplest portable backup of one
  experiment.

The server never claims that copying only `slither.db` is a complete backup
after checkpoint-v3 file storage begins. Stage 2/7 test interrupted backup,
concurrent pruning, missing files, orphan cleanup and restore from both cold
and hot backup sets.

### SQLite durability and the commit/acknowledgement window

The recommended production baseline is SQLite WAL mode with
`synchronous=FULL` for every transaction that changes a current checkpoint,
run identity, pin, imported state, history, or Hall-of-Fame reference.
“Durable” means SQLite has reported that FULL commit complete; it does not mean
merely that a JavaScript promise resolved before bytes reached the filesystem.
Checkpoint population bytes are already durable in their final managed file,
so this transaction contains only metadata and references. The target-VM
barrier benchmark uses this mode. It may be optimized without weakening that
promise; changing to a weaker power-loss contract requires explicit review
rather than a hidden performance tweak.

The database current pointer is the crash-recovery authority:

- failure before commit leaves the old pointer current;
- commit success followed by persistence-worker death or a lost reply leaves
  the new pointer current on restart;
- if the reply is lost, stale, or cannot be reconciled while Rust still holds
  the old world, the live engine faults and the supervised process restarts
  rather than advancing with Rust/database identities split;
- commit reply followed by process death before Rust's in-memory swap also
  resumes the new committed checkpoint;
- only after Rust accepts the matching operation/transition epoch may it swap
  the staged world and publish dependent success/lifecycle events.

Stage 3 proves only the minimum publication invariant: SQLite never points to
a partial/missing new file, and Rust never announces a checkpoint before the
matching metadata commit. The full failpoint matrix—worker/process death before
and after commit/reply/swap, WAL failure, orphan cleanup, missed replies and
retries—belongs in Stage 6B and Track 7D before production cutover.

### Owner-selected initial retention policy

- keep the latest valid resumable checkpoint for the current run;
- keep eight recent automatic checkpoints total, including that latest one;
- keep one milestone every 25 generations, with at most twelve milestones not
  already counted as recent;
- keep the latest resumable anchor for the two most recent prior runs;
- keep every explicitly pinned checkpoint until the owner unpins/deletes it;
- never let automatic pruning touch a downloaded export file;
- cap unpinned automatic checkpoint files at a configurable 4 GiB globally;
- when the byte cap is exceeded, remove the oldest unpinned milestones first,
  then old recent checkpoints, while retaining the current latest plus at
  least one predecessor and reporting the reduction;
- if the latest checkpoint alone cannot fit the configured safety budget,
  reject the unsafe configuration before the run becomes current rather than
  silently running without recoverability.

The initial 4 GiB cap includes every unpinned automatic class, including the
two prior-run anchors. The protected automatic minimum is the current latest,
one current predecessor, and those two prior-run anchors. Before accepting a
configuration/new run, the estimator proves that protected set fits the cap;
if reality later exceeds it, the engine pauses/faults the next durable
transition and asks for an explicit retention/budget choice rather than
deleting a protected anchor.

Pre-v3 rows are never inferred to be disposable merely because they lack a new
`pinned` flag. Unless their old boundary metadata proves they were ordinary
automatic checkpoints and the owner-selected policy covers them, classify them
as manual/legacy protected. Inactive staged-import/checkpoint metadata and
managed files carry an operation ID and expiry; startup verifies that they are
neither current nor referenced before garbage-collecting them. Repeated failed
imports therefore cannot create an unlimited hidden class.

Health reports stored/raw-equivalent bytes and selected encodings separately
for latest/recent, milestones, prior-run anchors, pinned checkpoints, Hall of
Fame, history, WAL, freelist, and temporary files. Pinned data is deliberately
user-managed and can exceed the automatic budget; the server warns or rejects
a new pin when free disk
would become unsafe, but it never resolves that condition by deleting an
existing pin.

Ordinary Export produces a temporary downloadable archive and does not leave
another permanent managed checkpoint. A separate explicit “Pin checkpoint”
action marks a server-side managed checkpoint permanently retained. Existing
`/api/save` compatibility is mapped to one clearly named operation rather than
silently creating an unbounded third category.

Per-generation adaptive stored size cannot be claimed before measuring real
genomes. The planning envelope below shows what one raw population would use
at two example compression ratios; incompressible payloads use the raw column,
and actual container/state metadata adds a small separately recorded amount.

| Workload | One raw population | At hypothetical 1.25:1 | At hypothetical 2:1 |
|---|---:|---:|---:|
| P0 | 2.82 MiB | 2.26 MiB | 1.41 MiB |
| P1 | 15.40 MiB | 12.32 MiB | 7.70 MiB |
| P2 | 84.53 MiB | 67.62 MiB | 42.27 MiB |
| P3 | 461.10 MiB | 368.88 MiB | 230.55 MiB |

Twenty current-run automatic snapshots (eight recent plus twelve milestones)
would contain the following raw population bytes before compression:

| Workload | Twenty raw populations | At hypothetical 1.25:1 | At hypothetical 2:1 |
|---|---:|---:|---:|
| P0 | 56.4 MiB | 45.1 MiB | 28.2 MiB |
| P1 | 308 MiB | 246 MiB | 154 MiB |
| P2 | 1.65 GiB | 1.32 GiB | 0.83 GiB |
| P3 | 9.01 GiB | 7.21 GiB | 4.50 GiB |

Including the two owner-selected prior-run anchors gives a count-only maximum of 22
unpinned automatic snapshots when none of those anchors duplicates the current
set:

| Workload | Twenty-two raw populations | At hypothetical 1.25:1 | At hypothetical 2:1 | Estimated automatic state after pruning |
|---|---:|---:|---:|---|
| P0 | 62.0 MiB | 49.6 MiB | 31.0 MiB | Retained evolved/fresh codec evidence gives 47.8–53.0 MiB for 22 weight payloads, plus container/state metadata |
| P1 | 338.8 MiB | 271.0 MiB | 169.4 MiB | Retained evolved projection gives 257.8 MiB for 22 weight payloads, plus container/state metadata |
| P2 | 1.82 GiB | 1.45 GiB | 0.91 GiB | Retained evolved projection gives 1.51 GiB for 22 weight payloads, plus Hall-of-Fame/container/state data |
| P3 | 9.91 GiB | 7.93 GiB | 4.95 GiB | Byte cap removes all milestones; eight recent plus two anchors project to 3.75 GiB of evolved weight payload |

Retained Stage 2 development-machine evidence measures P0 archive-v1 weight
payloads at 2.1709 MiB for the evolved fixture and 2.4101 MiB for the fresh
fixture, or 47.8–53.0 MiB for 22 payloads. The accelerated P0 retention fixture
physically materializes the evolved-volume files and metadata; the P1, P2 and
P3 figures above are indexed projections from retained evolved codec artifacts.
They are not target-VM or full checkpoint-v3 measurements.

For scale, 480 generation-summary records (eight hours of 60-second rounds) are
only about 26.3 KiB before table overhead/compression. Fifty unique
Hall-of-Fame genomes add about 2.57 MiB raw for the default brain or 76.8 MiB
raw for the P2/P3 large brain before compression. Those values are reported
beside—not hidden inside—the automatic checkpoint estimate. Pinned checkpoints
remain deliberately outside the automatic bound because only the owner can
decide when to remove them.

The 1.25:1 and 2:1 columns are planning examples, not claimed results. Stage 2
records actual P0/P2/P3 compression ratios and encode/decode times. The 4 GiB
cap is what bounds P3 if the count-based retention limits alone would not. Prior-run
anchors, pinned data, compact history, and Hall-of-Fame storage are reported
separately so a small automatic number cannot hide an unlimited category.
After an overnight run, automatic current-run checkpoints converge to this
retained set rather than retaining all 480 generation copies; P3 is further
limited by the byte cap. Pinned and Hall-of-Fame choices remain separately
visible rather than being falsely included in that bound.

These owner-selected values are configurable and Stage 2 must check them
against measured P0/P1/P2/P3 checkpoint sizes and the VM's real free-disk
behavior before pruning is enabled. “Pin checkpoint” and “Export archive” are
separate user-visible operations. No migration-time pruning of existing user
data starts before the selected rules and file/metadata references have been
verified.

### History and Hall-of-Fame growth

The current persistent/history inventory is:

- eight numeric generation-summary fields used by charts and evolution
  diagnostics;
- the latest rolling 100 summaries copied into each checkpoint metadata
  object;
- one full best-genome JSON object embedded in generation checkpoint metadata;
- another full best-genome JSON object in `hof_entries`;
- graph/settings/update data repeated in every checkpoint;
- no persisted per-step telemetry series.

The intended use of each category is explicit:

| Data | Why it exists | Retention meaning |
|---|---|---|
| Eight-field generation summary | Charts, species/network trends, diagnostics, and long-term experiment analysis | Preserve every generation compactly; not population-sized |
| Current generation-boundary population plus RNG/allocator state | Exact version-scoped resume | Governed by checkpoint retention; not called “history” to hide its size |
| Pending/latest Hall-of-Fame event | Closes the checkpoint-to-Hall-of-Fame crash window | Commit atomically, then reference the durable Hall-of-Fame entry |
| Hall-of-Fame metadata and genome | Ranking, inspection, resurrection, and later comparison | Separately reviewed Hall-of-Fame policy; packed/deduplicated weights |
| Graph/settings/config definitions | Interpret weights and reproduce the run | Store once per stable content version and reference it |
| Per-step timing/candidate/queue telemetry | Short-term performance diagnosis | Bounded in memory/log artifacts; not exact-resume state or an unbounded SQLite series |

New storage separates these meanings:

- preserve every per-generation eight-field summary in one append-only compact
  `generation_history` table keyed by run and generation, with the versioned
  56-byte record stored once; do not downsample it;
- store each summary once rather than copying the whole history into every
  checkpoint;
- let the browser request a chart window while export preserves the complete
  compact series;
- use the 56-byte binary record above, so one million generations are about
  53.4 MiB before database overhead/compression;
- store graph/settings definitions once per stable content/version key and let
  checkpoints reference them;
- store Hall-of-Fame metadata separately from weights;
- store a unique Hall-of-Fame genome as packed adaptive Float32 once, keyed
  by a domain-separated content hash covering graph architecture key, ordered
  layout version/digest, weight count, numeric encoding version, and logical
  Float32 bytes—not bare weight bytes—and let entries/checkpoints reference it;
- never add high-frequency per-step diagnostics to exact-resume state.

The live database does not recompress and rewrite a growing history BLOB on
every generation: one small fixed record is appended transactionally with the
generation checkpoint. The outer archive compresses the complete
`history.bin`. Stage 2 measures SQLite row overhead against optional immutable
compressed history segments (for example, sealing a few thousand records at a
time). Segment compaction is adopted only if it materially saves database
space without losing a record or delaying generation commits; it never changes
the full-resolution history policy.

The owner-selected initial Hall-of-Fame retention is the best 50 unique
run-scoped genomes plus explicitly pinned entries. Compact metadata for older
entries is preserved where inexpensive. Every eight-field generation summary
is preserved without downsampling. No existing Hall-of-Fame data is pruned
until compatibility migration, ranking/tie behavior, packed references and
the selected rule have been verified against copied user data.

### SQLite and managed-file space reclamation

New checkpoint-v3 payload deletion unlinks immutable files and returns
filesystem space immediately; it does not depend on SQLite vacuuming. SQLite
contains only small metadata/history/index/reference rows for new checkpoints,
so ordinary pruning uses short transactions and WAL growth should no longer
scale with population bytes.

Stage 2 still inventories the existing database because old raw
`snapshot_genomes`, combined blobs and repeated JSON may already occupy many
gigabytes. The migration/maintenance rules are:

- measure database, WAL, freelist, managed-file, temporary and orphan bytes
  separately;
- use passive WAL checkpoints from the persistence worker and derive a
  configurable high-water from measured small metadata transactions and any
  active legacy migration, not from checkpoint population size;
- do not automatically rebuild an existing database merely to enable
  incremental auto-vacuum;
- after verified legacy migration, deleting old rows makes pages reusable even
  if the file does not shrink;
- make full `VACUUM` an explicit offline legacy-database maintenance action
  that stops the simulation, validates a complete database-plus-file backup,
  and proves enough free disk for SQLite's temporary copy;
- measure that action on an overnight-sized legacy fixture in Stage 7 rather
  than placing it on the Rust-engine critical path;
- never let normal retention or maintenance unexpectedly duplicate a
  tens-of-gigabytes database.

### Existing-save compatibility

The following readers remain bounded and read-only:

- current v2 parent plus `snapshot_genomes` rows;
- legacy gzip `genomes_blob` records;
- legacy format-null/0 parent rows whose population is embedded in
  `payload_json` when no `genomes_blob` exists;
- current user-exported JSON population files;
- any older compressed population archive found in the owner's saved files and
  identified during Stage 2 inventory.

Legacy save *files* go through the same direct upload route. The browser sends
their bytes unchanged; server/Rust format detection, spooling, safety limits,
and staging do the conversion. A legacy JSON reader parses incrementally from
disk and converts one genome at a time; it does not reinstate a 50 MiB generic
request limit or a population-wide browser object. Legacy combined gzip remains
subject to its documented 512 MiB compressed/decompressed and per-genome
limits unless a safer measured migration reader is approved. By contrast,
current v2 child rows and database `genomes_blob` values are reached only by
startup/resume or an explicit copied-database migration operation; they are not
presented as files the browser can upload.

Current v2 database compatibility iterates `snapshot_genomes` rows and releases
each source BLOB after copying/validating it into the staged Rust population;
it must not use the current `.all()` whole-population path. For a legacy
SQLite `genomes_blob`, the persistence worker reads bounded BLOB slices (for
example SQLite `substr` chunks) into Rust's streaming gzip decoder instead of
materializing the 512 MiB BLOB plus another 512 MiB decompressed buffer.
For format-null/0 `payload_json`, it reads bounded byte slices such as
`substr(CAST(payload_json AS BLOB), offset, length)` into a Rust streaming JSON
sequence visitor that holds at most one bounded genome. It never selects the
complete population-sized TEXT value into JavaScript.

The retained Stage 2 probe on `better-sqlite3` 11.10.0 reconstructed
2,527,124-byte BLOB and multibyte UTF-8 TEXT fixtures exactly through
65,536-byte JavaScript-visible slices. It could sample memory only at
whole-role boundaries and therefore did **not** prove that SQLite or
`better-sqlite3` avoids a population-sized native allocation while evaluating
`substr`. The artifact explicitly leaves this query path unauthorized for
production. Stage 6B must instead use a separately reviewed native
incremental-BLOB path, or propose a strict legacy compatibility ceiling with
evidence and review; it must not promote the disposable query merely because
the returned JavaScript Buffers were bounded. A4/A8 record peak memory for v2
rows, database `genomes_blob`, old parent `payload_json`, compressed files, and
legacy JSON separately.

Compatibility readers never update or delete a legacy row/BLOB/file. A
dedicated conversion opens the source database read-only and writes a separate
v3 destination, so the complete source file remains untouched. Normal writable
startup from a copied/current database may add new schema and v3 rows, which
naturally changes the SQLite file and WAL; its enforceable promise is that the
legacy logical records remain present and hash-identical, not that the whole
database file remains byte-identical after SQLite opens it.

A successful legacy import can immediately export archive v1 as its migration
path, but provenance/completeness fields remain honest. A source lacking saved
RNG, allocator, full history, or Hall of Fame becomes a v1
`legacy-population-import`, not an exact-resume experiment. The compatibility
behavior installs its population/graph/settings at a newly
committed zero-state boundary under the active run identity, retains the source
seed only as provenance, and initializes missing current-run state by the
normal versioned reset rules. Unknown or incompatible graph ordering, archive
version, RNG version, or record layout fails clearly and leaves current state
untouched. A reader may be retired only after its exact supported scope,
warning period, migration path, and the owner's actual databases/save files
have been inventoried and any desired saves converted.

### Route and startup behavior

The thin Node layer retains health, graph preset, Hall-of-Fame, resurrection,
settings, Reset, and New Run surfaces. The proposed v1 HTTP transfer surface
is deliberately small:

- `GET /api/export/latest` is the one ordinary attachment request; it binds a
  checkpoint, completes the temporary archive before headers,
  then streams that exact file;
- `POST /api/import/archive` accepts the selected archive as its raw binary
  body and returns only a small result.

Route spelling may be aligned with the existing router during implementation,
but these two paths must not collapse back into a browser-buffered population
response or a generic JSON upload. Old endpoints may return a clear migration
response or bounded compatibility behavior; they never direct a new archive
through generic JSON.

Startup is:

1. The main Node thread parses config and starts the persistence worker; only
   that worker opens SQLite.
2. The persistence worker selects the requested current v3, current v2, or
   supported legacy checkpoint without changing its logical source records.
3. For v3, the worker returns the controlled managed path and expected
   metadata/root; Rust streams that immutable file directly. Only v2/legacy
   database sources require the worker's bounded source spool. Population
   bytes never cross the main Node isolate.
4. Rust validates versions, lengths, logical hashes/root, graph ordering,
   settings, identity, RNG, allocator, population, history, and Hall-of-Fame
   references.
5. Rust constructs the sole authoritative world within the memory-admission
   budget.
6. Node marks startup healthy only after Rust returns the matching current
   identity.
7. Only then does the simulation accept successful player/game commands.

The owner-selected normal `latest` recovery rule is newest valid retained boundary,
not blind trust in one corrupt pointer. The worker/Rust pair first validates
the current target, then—only if it fails—checks retained eligible checkpoints
newest to oldest within the bounded retention set. If an older valid boundary
is found, it does **not** continue under the old run ID into already existing
future generations. A FULL transaction creates a new recovery-branch run ID
whose provenance names the failed run, recovered generation/root, and abandoned
suffix; it reuses immutable content by digest, copies/references compact history
only through the recovered generation, and makes the branch checkpoint current.
The original corrupt record, its managed file if present, and future rows
remain preserved and ineligible for automatic continuation; corruption is not
misclassified as an orphan eligible for deletion. Health/welcome/logs plainly
report the failed ID, branch ID, recovered ID, and lost generation range. If
no candidate validates, Node remains in explicit startup-fault/health-only mode
instead of entering a blind supervisor restart loop.

An explicit `--resume <id>` is different: that exact ID either validates or
startup fails with its reason. It never silently substitutes another
checkpoint.
Tests cover a bad current pointer, bad newest payload, several corrupt
candidates, a valid older candidate with colliding future generations, branch
continuation into the next generation, no valid candidate, and explicit-resume
rejection.

Exact replay remains version-scoped. New archive v1 supports exact continuation
at its declared generation boundary. Cross-build/platform numeric guarantees
remain those defined by the determinism section, not a universal bit-identity
claim.

### End-to-end archive acceptance requirements

The archive feature is not complete until all ten tests pass through the real
browser, HTTP server, Rust engine, SQLite database, archive, and restore path.

**A1 — Large ordinary download.** Export both a realistic population whose old
decimal JSON representation is substantially over 50 MiB and a larger fixture
whose *actual archive file* is over 50 MiB. The browser uses the
original direct download request and receives one `.slither-save`
attachment through its normal download system. The saved file passes
independent archive validation. If generations, checkpoints, Reset, New Run,
or pruning continue while preparation runs, the response filename, manifest,
and bytes all remain bound to the exact checkpoint selected when that request
started.

**A2 — Browser export memory.** Run small, P2, and at least one larger fixture
under browser automation with precise memory reporting. The large-minus-small
renderer JavaScript-heap peak is at most 64 MiB, retained heap after collection
differs by at most 16 MiB, and total browser/renderer/network-process private
memory rises by at most 256 MiB. None of those metrics may grow in proportion
to declared decompressed population bytes. Source tracing proves no population
object, population string, or population Blob exists.

**A3 — Browser import memory.** Upload the same on-disk files as raw `File`
bodies. The large-minus-small JavaScript-heap and browser-process limits match
A2. Browser code never reads the file contents and receives only small
progress/result/error messages. The actual-archive-size-over-50-MiB fixture
must succeed, proving the route does not inherit the generic JSON parser or its
50 MiB ceiling.

**A4 — Server memory.** Instrument archive generation, temporary-file write,
download, upload spooling, validation, decompression, managed-checkpoint file
publication, the small SQLite metadata commit, and Rust candidate construction
separately. The provisional target is at most 256 MiB archive I/O overhead
outside the current plus staged engine and total process RSS below 12 GiB;
Stage 2 replaces those provisional values with configurable limits derived
from the 16 GiB VM measurements. Exceeding the active estimate is rejected
before state construction, not discovered by OOM. Repeat for v2 row iteration,
database `genomes_blob`, format-null/0 database `payload_json`, old compressed
files, and legacy JSON. Main Node event-loop and control latency remain measurable
while the persistence worker does synchronous SQLite work: event-loop delay is
at most 20 ms p95 and 50 ms p99, local `/health` response is at most 100 ms
p95, and player/RL action-to-step latency continues to satisfy the 100 ms p95
LAN gate. A P2 export-source assembly overlaps a generation boundary and still
meets the one-second p95/two-second maximum FULL-durability checkpoint barrier.

**A5 — Adaptive packed encoding.** For every fresh/mid-evolution P0/P2/P3
numeric payload, record raw packed bytes, shuffled-Zstandard candidate bytes,
the selected encoding, archive/container overhead, encode/decode throughput,
and equivalent decimal-JSON bytes. Verify bit-exact decoding and bounded
scratch memory for both encodings. Select compressed only when its measured
stored form is smaller; otherwise select raw packed. No valid archive fails
merely because high-entropy weights save less than ten percent. The selected
numeric representation must not materially exceed raw packed bytes plus small
declared container metadata, and the complete large fixture must remain at
least four times smaller than equivalent decimal Float32 JSON. Report actual
ratios rather than presenting codec use as a guaranteed ratio.

**A6 — Complete round trip.** Export/import preserves required settings,
updates, graph and ordered layout digest, every genome weight bit, recurrent
state when present, generation/history, best state, Hall of Fame, RNG state,
allocator state, run identity, completed-step boundary, and declared
continuation semantics. The next generation/event sequence matches a direct
continuation within the versioned deterministic contract. Ordinary Export is
one self-contained generation-boundary experiment; exact import restores its
identity when non-conflicting, and its Hall of Fame is restored within that
run rather than merged globally.
With live clients, the successful replacement also advances `worldEpoch`,
rejects old queued actions/tokens/results, sends one reliable state-replaced
result, keeps sockets open, and places clients in `awaitingRejoin` without
restoring archived connection state. Repeating the identical archive reuses
identical content/keys idempotently.

**A7 — Failure atomicity and transport limits.** Truncation, corrupted entry
or logical hash/root, unsupported version, false declared decompressed size,
oversized decoder window, decompression bomb,
duplicate/path-traversal/link/device/sparse/PAX entry, incompatible graph
ordering, missing record, and interrupted upload all fail before replacement.
The same run/generation/record key with a different root digest is an atomic
identity conflict, not an overwrite or merge.
HTTP cases include an over-limit `Content-Length`, a body longer or shorter
than its stated length, no `Content-Length`, chunked transfer crossing the
archive limit, disconnect at every spool boundary, a connected upload that
makes no progress past the idle timeout, and restart during upload. Resource
cases include temporary-file quota exhaustion, SQLite/WAL
admission failure, insufficient disk reserve, process-memory rejection, and a
second concurrent archive job receiving `busy` without creating a spool.
Persistence failpoints cover worker death/error before commit, after FULL
commit but before reply, after reply but before Rust swap, and after swap but
before public success; a lost reply can never leave a live old world advancing
against a new database pointer.
Export cases include encoder/container-completion/selected-write-validation
failure before `.ready`, cancellation at every download boundary, an opened
download that stops consuming bytes past the idle timeout, server restart with
`.partial`/`.ready` files, two concurrent direct export requests, and
generation/checkpoint/prune changes while the first request is bound. Every
case leaves the prior engine digest, current database pointer, settings, graph,
history, Hall of Fame, controller
leases/epochs/queued actions, and user source file unchanged; no unreferenced
job row, staged row, orphan managed candidate, `.partial`, or `.ready` file
survives the documented cleanup window. Referenced corrupt evidence is
preserved under the recovery rule and is not treated as an orphan.

**A8 — Legacy migration.** This has two distinct entry paths. For database
compatibility, start the server from disposable copies of databases containing
v2 child rows, legacy `genomes_blob`, and format-null/0 population-sized
`payload_json`; read them through bounded row/BLOB/TEXT-slice readers. A
read-only conversion leaves the whole source copy unchanged; a writable-startup
test instead proves every legacy logical record remains hash-identical after
new schema/rows are added. These are database startup/resume migrations, not
pretend browser uploads. For file compatibility, directly upload fixtures for
current JSON exports and every identified older compressed owner-save format
through their bounded server-side readers. Each supported case re-exports and
re-imports as archive v1. Sources missing RNG/history/Hall-of-Fame/allocator
state must advertise partial provenance and compatible population-import
semantics rather than exact continuation. Unsupported cases name the exact
limit or incompatibility without destructive migration. The owner's real
databases and save files must be inventoried before any compatibility reader
is narrowed or retired. Older exact boundaries that conflict with a later
suffix use the selected explicit branch behavior; they never delete that
suffix silently.

**A9 — Overnight disk budget.** A deterministic accelerated fixture represents
at least the same generation count and bytes as an eight-hour 60-second-round
run for P0, P2, and the P3 measured-capacity case. It includes checkpoints, history,
Hall of Fame, managed files, SQLite/WAL, pruning, orphan cleanup and legacy
compaction. The managed directory plus database/WAL remains inside the selected
configurable budget, pinned data and downloaded exports are untouched, and
deleted checkpoint-file space is reclaimed. Every retained current, prior-run,
milestone, and pinned anchor restores and re-exports successfully after
reference-aware pruning.

**A10 — Browser usability.** During supported export and import, animation,
controls, progress/error UI, and a separate control connection remain
responsive; the tab does not hang, crash, or get killed. File UI work creates
no population-dependent main-thread task and no file-processing task longer
than 100 ms; at least 95 percent of display intervals remain at or below 40 ms,
player/RL action-to-step remains below 100 ms p95, and assignment/reclaim has
zero silent loss. A companion control case delays/suppresses browser sensors
and display frames while pointer and boost state change; fresh steering and
boost release must still be sent and remain eligible for the next fixed-step
drain. The test covers the laptop browser over LAN as well as local automation.

Static and integration checks identify the normal browser export/import call
graph and fail if it contains population-wide `response.json()`,
`response.text()`, `FileReader.readAsText()`, file `.text()`/`.arrayBuffer()`,
`Response.body`, `ReadableStream.getReader()`, `File.stream()`, manually
accumulated chunks, population `JSON.parse()`/`JSON.stringify()`,
`Array.from(weights)`, or a browser-created population `Blob`. The positive
allowlist is also checked: export immediately activates one ordinary
attachment navigation/link and never fetches the archive; import passes the
original `File` directly to `XMLHttpRequest.send()` and reads only small upload
progress plus the small result. Route tracing also proves the binary upload
never calls the generic JSON body reader. Small JSON used by unrelated
settings, graph editing, health, or error responses is not falsely banned.

## LAN and Debian deployment

### Networking

- `host`, `uiHost`, `publicWsUrl`, environment variables, and CLI overrides
  keep their trusted-LAN meaning.
- `0.0.0.0` and explicit VM LAN addresses remain valid.
- Browsers continue to derive the simulation host from the UI hostname when
  appropriate.
- The server does not advertise router port forwarding as safe.
- No authentication/TLS/public hardening project is inserted into the Rust
  migration.

### Development and production UI

- Vite remains the browser development tool with LAN-capable HMR.
- For the Debian VM, add a production path that serves the built browser
  assets without leaving a Vite development process running permanently.
- The simplest target is the existing Node interface process serving `dist`
  alongside API/WebSocket routes, while preserving separate dev ports.
- The launcher prints usable LAN URLs and the active Rust thread count.

### Build target

- Primary deployment target: x86_64 Linux GNU on Debian.
- Primary CPU profile: Ryzen 7 2700/Zen+ with runtime feature detection.
- Windows x86_64 remains a developer/test target.
- Build Rust in release mode.
- SIMD paths require scalar/reference parity and safe runtime dispatch.
- No WASM simulation fallback is required.

### Service operation

The final deployment stage provides:

- one documented build command;
- one documented start command;
- graceful SIGTERM checkpoint/boundary behavior;
- a tested systemd unit for the Debian VM with `Restart=on-failure`, bounded
  restart delay/rate limiting, the configured data directory, and no restart
  loop that can mutate an invalid database;
- a plain manual-start alternative for diagnosis, without claiming it provides
  automatic crash recovery;
- log rotation guidance;
- database and config paths outside generated build output;
- health output showing engine version, worker count, tick, speed ratio,
  memory, and fault/overload state.

## Source-to-Rust behavior map

This table prevents the “delete TypeScript, then guess what it did” failure.
The exact Rust filenames may be adjusted during implementation, but every
source responsibility requires a named destination and tests before cutover.

| Current source | Behavior to study and fixture | Proposed Rust destination |
|---|---|---|
| `src/config.ts` | Complete defaults and nested settings | `engine/config.rs` |
| `src/protocol/settingDefinitions.ts` | Ranges, types, live/reset behavior | shared generated contract plus `engine/config.rs` |
| `src/rng.ts` | xorshift, seed derivation, Gaussian spare state | `engine/rng.rs` |
| `src/protocol/sensors.ts` | v3 names, offsets, lengths | `engine/sensor_layout.rs` |
| `src/sensors.ts` | Every scalar/bin formula and corrected body query | `engine/sensors.rs` |
| `src/brains/graph/schema.ts` | Graph wire schema | `engine/graph/schema.rs` |
| `src/brains/graph/validate.ts` | Graph constraints and errors | `engine/graph/validate.rs` |
| `src/brains/graph/compiler.ts` | Topology, ports, parameter offsets/key | `engine/graph/compile.rs` |
| `src/brains/graph/runtime.ts` | Node execution and output mapping | `engine/graph/eval.rs` |
| `src/brains/ops.ts` | Dense/MLP/GRU/LSTM/RRU math/state | `engine/brain/*.rs` |
| `src/brains/stackBuilder.ts` | Default graph creation | shared fixture/generator plus Rust validation |
| `src/mlp.ts` | Genome layout, initialization, crossover, mutation | `engine/genome.rs`, `engine/evolution.rs` |
| `src/snake.ts` | Spawn, body, scores, boost, movement, food, death | `engine/snake.rs`, `engine/physics.rs` |
| `src/spatialHash.ts` | Current collision intent and known defects | `engine/spatial/segments.rs` |
| `src/world.ts` pellet grid | Pellet add/remove/query semantics | `engine/spatial/pellets.rs` |
| `src/bots/baselineBots.ts` | Bot slots, strategies, states, respawn RNG | `engine/baseline.rs` |
| `src/world.ts` control path | Stable observation and source priority | `engine/control_step.rs` |
| `src/world.ts` collision path | Substeps and intended body collision | `engine/collision.rs` |
| `src/world.ts` generation path | Fitness, selection, history, HoF | `engine/evolution.rs` |
| `src/world.ts` God Mode/import/resurrection | Live mutation semantics | `engine/commands.rs` |
| `src/sim/SimCore.ts` | Fixed-step, stats, reset/New Run | `engine/runtime.rs` |
| `src/serializer.ts` | Binary frame v1 | `engine/frame/v1.rs` |
| `server/controllerRegistry.ts` | Assignment/action semantics and defects | `engine/controllers.rs` |
| `server/simServer.ts` | Command order, durability, event timing, and the defective welcome refresh that serializes a whole world for one byte-length field | thin `server/simServer.ts` plus `engine/runtime.rs`; latest packed-frame length is routed as small metadata |
| `server/checkpoint.ts` | Exact boundary payload | `engine/checkpoint.rs` |
| `server/snapshotTypes.ts` | Current v2 wire/storage shapes and omitted transfer state | shared bridge/archive contract plus compatibility reader |
| `server/persistence.ts` | SQLite, raw child rows, growth, and legacy compatibility | thin metadata/file-inventory persistence worker, bounded legacy iterator, and Rust managed-checkpoint/archive codec |
| `src/storage.ts` and `src/main.ts` save paths | Browser buffering/parsing and local metadata substitution | direct browser download/upload initiation only |
| `server/protocol.ts` | Protocol 2 validation/wire types | remains TypeScript; optional lease fields added |
| `server/wsHub.ts` | Connection routing/backpressure | remains TypeScript, with priority queues |
| `server/httpApi.ts` | HTTP routes | remains TypeScript, calling engine commands |
| `src/main.ts::sendPlayerAction`, `onSensors`, and pointer/button handlers | Current sensor-gated sending, latest desired player state, screen-space direction, immediate meaningful-change send, periodic resend, and RL/client distinction | remains browser TypeScript; latest-value commands route to `engine/controllers.rs` |
| Remaining `src/main.ts` | UI, reconnect, settings | remains browser TypeScript |
| `src/render.ts` | Frame consumption and drawing | remains browser TypeScript |

For each row that moves, the implementation record must name:

- source functions and lines reviewed;
- intended behavior;
- known bug intentionally corrected;
- fixture/test proving the Rust result;
- any numeric tolerance;
- measured allocation/performance result;
- whether the TypeScript runtime call site is still production-active.

## Verification strategy

### Rule 1: Characterize before deleting

No current behavior-bearing function is removed before:

1. its intended behavior is written as a fixture or test;
2. known defects are separated from behavior to preserve;
3. the Rust replacement passes;
4. an integrated Rust step uses it;
5. the production cutover no longer calls it.

### Rule 2: Compare complete stages

Tests compare stage boundaries, not only final round scores:

- normalized config;
- compiled graph and parameter offsets;
- initialized genomes;
- spawn state;
- sensor vectors;
- neural outputs and recurrent state;
- movement proposal;
- food claims;
- collision candidate set;
- committed deaths/scores;
- generation fitness and selected parents;
- next population;
- frame bytes;
- checkpoint manifest, managed file, metadata/current pointer, history
  records, and restore digest.

This makes a mismatch local enough to diagnose.

### Rule 3: Known bugs get correction fixtures

The following must not use the current broken result as a golden value:

- production body/hazard sensors;
- collision-grid truncation;
- simultaneous collision array-order bias;
- unsafe overlapping spawn;
- ignored collision range/high-speed crossing;
- immediate controller takeover;
- simulation-tick ownership timeout;
- unreliable assignment under backpressure;
- stale player/RL control through catch-up steps;
- browser-player action transmission gated by incoming sensors;
- external joins advancing evolution/world RNG;
- stale absolute mouse target;
- welcome refresh serializing the complete world for frame-length metadata;
- browser population buffering and the generic 50 MiB import limit;
- unbounded automatic checkpoints and duplicate decimal Hall-of-Fame genomes.

For these, a plain-language expected rule is written first and both the
temporary reference path and Rust path are tested against it where useful.

### Rule 4: Numeric tolerances are explicit

- Integer IDs, counts, event order, RNG state, graph topology, parameter
  counts, controller ownership, and checkpoint metadata are exact.
- Rust reference neural math is compared tightly with TypeScript for one step.
- SIMD and parallel neural paths use a documented absolute/relative tolerance.
- Long-run floating divergence is evaluated through invariant/event checks and
  version-scoped replay, not a false universal bit-identity promise.

### Rule 5: Thread-count tests

For the same seed/config/action log and engine build:

- one worker and selected production worker count must produce the same
  discrete event ordering and identities;
- collision outcome, food winner, death awards, generation membership, and
  controller ownership must be identical;
- numeric state must meet the defined deterministic/tolerance contract;
- recurrent state stays with its stable population slot.

### Rule 6: Black-box clients remain valid

Run the current browser and a Protocol 2 bot client against the Rust engine
without giving either access to internal Rust objects. Validate:

- handshake/welcome;
- frames and stats;
- join/assign/sensors/action;
- death/reassignment;
- settings;
- God Mode;
- Reset/New Run;
- save/export/import/resurrection;
- graph presets and Hall of Fame;
- LAN CORS and URL discovery;
- disconnect/reclaim;
- fault and overload status.

## Performance and capacity plan

### Baseline first

After plan authorization and before optimization, collect current production data
on:

- the development machine;
- the Debian VM with exactly eight allocated threads and 16 GB;
- default native serial mode;
- current `--mt` at bounded worker counts;
- JavaScript diagnostic mode;
- browser connected and disconnected;
- RL bot connected and disconnected;
- the current browser-buffered import/export path plus a small offline
  prototype of the proposed direct-transfer/archive codec;
- current checkpoint, Hall-of-Fame, WAL, and database bytes per
  generation;
- compression ratio and encode/decode cost on real default and large-brain
  genome data.

The baseline is evidence, not an acceptance of the current architecture.

### Standard scenarios

Every end-to-end benchmark uses real different genomes, real recurrent state,
real sensing, movement, collision, frames when specified, and the production
server boundary.

#### P0: Default

- 55 evolved snakes;
- 10 baseline bots;
- v3 16-bin sensors (83 inputs);
- 64/64 MLP, GRU 16, Dense 2;
- 3,500 pellets;
- 60 Hz physics/control;
- 1x speed;
- browser spectator and a separate run with browser player.

#### P1: Maximum population with default brain

- 300 evolved snakes;
- default brain and sensor layout;
- 10 baseline bots;
- normal pellets;
- 1x speed.

#### P2: Large brain

- 55 evolved snakes;
- 32-bin v3 sensors;
- five 256-unit MLP layers;
- recurrent hidden size 96;
- output 2;
- 1x speed.

Graph variants cover GRU, LSTM, RRU, Split, and Concat rather than measuring
only the default stack.

#### P3: Population plus large brain

- 300 evolved snakes;
- large custom graph representative of owner use;
- 32-bin sensors;
- 1x speed.

This is initially a capacity characterization. It becomes a real-time gate
only if baseline measurements show it is physically realistic on the VM or
after an explicit owner supported-envelope decision.

#### P4: Dense world and long bodies

- high pellet target;
- long snakes and repeated deaths;
- enough segments to exceed the current 200,000-entry failure point;
- browser follow and overview modes;
- collision, memory, frame, and LAN focus.

#### P5: External control

- one browser player;
- one RL bot;
- induced packet delay, jitter, brief disconnect, and frame backpressure;
- separately delayed/suppressed browser sensor delivery while pointer,
  steering, boost press, and boost release continue;
- browser-player periodic-send candidates at 30 Hz and 60 Hz, while the RL bot
  remains observation-driven;
- default and stressed simulation loads;
- 1x and accelerated simulation.

#### P6: Accelerated training curve

- default and large-brain populations;
- requested speed 1x, 2x, 4x, 8x, and 12x;
- no display client, then one spectator;
- achieved multiplier and controller observation throughput recorded.

#### P7: Soak

- at least 30 real minutes;
- repeated generations, deaths, controller reconnects, saves, and frames;
- memory growth, handle leakage, queue depth, database/WAL bytes, pruning, and
  deterministic fault behavior.

#### P8: Archive and overnight-equivalent persistence

- adaptive raw/compressed selection, with no numeric payload materially larger
  than raw packed and a large reduction from current decimal JSON;
- direct browser download/upload of default, P2, and a fixture whose actual
  archive body exceeds 50 MiB;
- actual browser heap, Node/Rust RSS, temporary/managed-file disk, SQLite/WAL,
  encode, decode, file publication, metadata commit, restore, prune, orphan
  cleanup and legacy-vacuum timings;
- an accelerated fixture representing at least eight hours of 60-second
  generations and the same total checkpoint/Hall-of-Fame/history bytes;
- corruption, interruption, and legacy-import cases from A1–A10.

### Measurements

For every scenario:

- completed steps per wall second;
- achieved/requested simulation ratio;
- fixed-step p50/p95/p99/max;
- subsystem timing;
- controller action-to-step latency p50/p95/p99/max;
- browser pointer/boost-change-to-send, send-to-server-accept, and
  accept-to-eligible-step latency, including sensor/display suppression;
- sensor-to-action round trip;
- frame rate/bytes/pack/parse/render time;
- Node event-loop delay;
- Rust worker count and CPU use;
- engine/Node/browser resident memory;
- allocations after warm-up where measurable;
- collision candidate/index counts;
- sensor cap hits;
- queue depth/coalesced/dropped data by class;
- generation configured seconds versus wall seconds;
- checkpoint raw/compressed bytes and ratio;
- archive encode/decode/download/upload/commit/restore throughput;
- database, WAL, freelist, temporary-file, history, and Hall-of-Fame bytes;
- browser JavaScript heap and renderer-process memory during file transfer;
- correctness failures, not only timing.

### Performance gates during migration

Performance is measured when each hot subsystem first exists, not saved for a
final optimization stage:

1. **After Stage 2 baseline:** publish current integrated P0/P2/P4/P5/P8
   results, including the count-one N-API pattern and database growth. These
   numbers are evidence, not targets for Rust to copy.
2. **After Stage 3 bridge foundation:** exercise the real idle and sustained
   Node↔Rust command/event path, persistence worker, wake/re-arm race, queue
   bounds, event-loop delay, latency, CPU, and memory. A coarse boundary is not
   accepted merely because no gameplay exists yet.
3. **During Stage 4 sensing and spatial indexing:** benchmark corrected dense
   body/pellet sensing, query/index load, capacity behavior, and allocation
   before and after each representation change.
4. **After Stage 4 heterogeneous inference:** benchmark the complete differently
   weighted due population with real recurrent state for P0, P1, P2, and P3.
   Report whole-pass time and memory. A shared-weight kernel batch is not
   evidence.
5. **During and after Stage 5 movement/collision/scalar work:** benchmark long
   bodies, dense pellets, collision-grid load, complete movement/collision
   phases, and then complete fixed-step p50/p95/p99, simulated seconds per wall
   second, subsystem time, control latency, and memory before parallelism.
6. **At Stage 6A real integration:** repeat through the actual Node process,
   frame v1, current browser, LAN player, and Protocol 2 trainer path.
7. **During Stage 6B and Track 7D durability:** run P8 after archive creation,
   managed-checkpoint retention and maintenance changes rather than
   inferring end-to-end behavior from a codec microbenchmark.
8. **During Track 7P parallelization:** compare one, four, five, six, and where
   useful seven calculation workers on the actual eight-thread VM. Add one
   parallel subsystem at a time and rerun deterministic/correctness gates.

### Initial required acceptance targets

These are proposed targets for review:

- P0, P1, and P2 sustain at least 0.98 simulated seconds per wall second at 1x
  over ten minutes on the target VM.
- After warm-up, supported P0/P1/P2 runs have `droppedWallDebt == 0`. The 0.98
  ratio cannot be achieved by discarding part of the requested schedule.
- From the playable start of one configured 60-second generation to the
  playable start of the next, P0/P1/P2 take no more than 62 wall seconds. This
  includes the required generation checkpoint, transition, and next spawn;
  database work cannot be reported separately to hide a long user-visible
  stall.
- The required generation-boundary persistence barrier is at most one wall
  second at p95 and two seconds maximum for P0/P1/P2 on the target VM, while
  Node event-loop delay remains at most 20 ms p95/50 ms p99, local `/health`
  remains at most 100 ms p95, and LAN action-to-step remains below 100 ms p95.
- P0/P1 fixed-step p99 is at or below 16.67 ms after warm-up; P2 may use the
  ratio/generation gate if rare p99 spikes do not accumulate debt.
- No authoritative step, collision segment, control transition, or required
  checkpoint is silently dropped.
- A browser/RL action accepted before a step begins is visible no later than
  that step’s control drain.
- LAN player action-to-authoritative-step latency is below 100 ms at p95 in
  P0/P1/P2, with a target below 50 ms.
- Under delayed or fully suppressed browser sensors, a pointer change and boost
  release leave the browser at the selected resend/rate-limit cadence, reach
  the server, and remain eligible for the next fixed-step control drain.
  Display-frame replacement or delay does not alter this gate.
- At least 95% of browser display-frame intervals are no longer than 40 ms
  (25 fps equivalent) under the agreed player workload, and control remains
  usable when display frames are deliberately replaced/dropped.
- Reliable assignment/reclaim results have zero silent loss under induced
  frame backpressure.
- The server process—including current/staged Rust state, Node, SQLite, and
  archive buffers—stays below 12 GiB RSS; admission rejects a request before it
  consumes the remaining 4 GiB VM/OS safety margin.
- After a ten-minute warm-up, the final twenty minutes of the 30-minute soak
  have RSS linear slope no greater than 1 MiB/minute and final RSS no more than
  64 MiB above the warm-window median. Every bounded queue remains below its
  configured capacity, temp-file bytes return to the expected active set, and
  retained managed-file/database bytes follow the owner-selected pruning
  envelope. Longer P7/P8
  runs repeat the same slope/plateau checks.
- P8 satisfies A1–A10 and the configured automatic persistence byte budget.
- Changing Rust worker count does not change discrete game outcomes.

The plan does not claim these targets have already been achieved or are already
physically demonstrated on the Ryzen 7 2700.

### Owner-selected supported-envelope rule

The UI currently permits extreme combinations: 300 snakes, five 256-unit
layers, 32 sensor bins, 25,000 pellets, and bodies up to 100,000 points. It is
not honest to guarantee every maximum simultaneously before measuring the
target CPU and RAM. P0, P1 and P2 are mandatory real-time targets on the Debian
VM. P3 is initially a measured capacity case rather than a real-time promise.

Selection as a mandatory target is not evidence that the Ryzen 7 2700 has
already achieved it. Stage 2 and Stage 7 report the actual result honestly. If
proper profiling and reasonable optimization still cannot meet a mandatory
target, report the limiting subsystem and measured capacity for owner review;
do not alter physics, drop collisions, degrade sensors, or silently lower the
workload to manufacture a pass.

After Stage 2 baseline and Stage 7 target-VM data:

1. publish combinations that meet real-time player guarantees;
2. publish combinations intended for faster/slower unattended training;
3. estimate memory/work before Reset;
4. warn clearly when a combination exceeds the measured real-time envelope;
5. reject only combinations that would violate memory/correctness safety;
6. never silently degrade sensors or collisions to make an unsafe setup run.

## Implementation stages

All checkboxes were intentionally open when Draft 4 was approved. Approval
authorizes implementation, but it does not complete any checkbox. A checkbox
may be completed only when its listed evidence exists. Passing unit tests or
finishing code is not, by itself, an exit gate.

The critical path is intentionally short:

```text
repair the reference
  -> measure the real failures and data growth
  -> establish the Rust state/bridge/graph/RNG/checkpoint spine
  -> port sensing plus whole-population inference
  -> port the complete fixed step
  -> connect generations/frame v1 to Node, browser, and RL (Stage 6A)
  -> run the first usable Rust game through the real browser and trainer
  -> complete direct archives/retention/compatibility (Stage 6B)
  -> optimize and accept the experimental Rust-authoritative game
  -> make Rust production, retire the old game, then consider optional protocols
```

Stages 1–2 may change the current TypeScript path only where needed to make it
a trustworthy reference and keep player/RL control usable during migration.
Stages 3–7 build forward beside that reference. Stage 8 changes normal startup
only after the experimental Rust path has passed the real Debian-VM, browser,
RL, persistence, and data-growth gates.

### Stage 1: Correct the record and repair the minimum trustworthy TypeScript reference

**Purpose:** Stop future sessions following the false kernel-only architecture,
and fix only the current defects that would either make the reference lie or
make the program unusable while Rust is being built.

**Documentation correction after this draft is approved:**

- [x] Correct active instructions and user documentation to say that Rust will
  own one authoritative game, Node is a thin interface, and the browser is a
  remote renderer/control surface.
- [x] Supersede the kernel-only ADR and mark the earlier recovery plan's false
  owner-attribution plainly; preserve a factual historical record without
  pretending the owner selected that design.
- [x] Preserve trusted-LAN operation and the separate Protocol 2 RL client as
  required product surfaces.
- [x] Record the approved plan revision and Git commit in the short
  implementation log. Keep that one factual record and do not invent owner
  decisions.
- [x] Preserve exact Git-history evidence before stating when the false
  documents were introduced or what the old controller/commit `8330065` did:
  record repository identity, commands, commit IDs, diffs, relevant full file
  contents, and raw output. Keep current-document proof separate from those
  historical claims.

**Minimum TypeScript reference repairs:**

- [x] Replace the broken body-sensor spatial-query adapter with the real query
  contract and add a real-body regression test for neural, baseline, and
  external observations.
- [x] Replace silent collision-grid truncation with a checked grow/rebuild at a
  safe boundary. If the configured world cannot be represented inside its
  memory limit, fault before committing the step.
- [x] Expose collision-grid capacity, current entries, peak entries, rebuilds,
  and admission/fault reasons in diagnostics.
- [x] Hold the last accepted player/RL input for 500 ms wall time, then use
  neutral steering and boost-off. Reserve disconnected ownership for 30 wall-
  clock seconds. Make both values configurable.
- [x] Never run the brain during grace. At expiry, perform one explicit
  external-to-neural ownership transition, invalidate the expired controller,
  and never mix its input with brain output. Keep these clocks independent of
  simulation speed and scheduler debt.
- [x] Reset the scheduler clock after asynchronous initialization so startup
  delay cannot appear as simulation debt.
- [x] Slice temporary TypeScript catch-up into bounded groups of complete
  fixed steps and return to the Node event loop between groups. With an
  interactive controller attached, service Node between every overdue step.
- [x] Refresh the current step/control identity at each fixed-step boundary so
  two legitimate observations in one old-style pump do not share a stale
  limiter state.
- [x] Prioritize assignment, reclaim, player/RL control, errors, import/export
  results, and lifecycle messages over replaceable display frames.
- [x] Retain at most the newest unsent display frame for each browser
  connection and make failure to enqueue/send a reliable message observable.
- [x] Keep player pointer input in screen coordinates and recompute steering
  from the newest camera/player state, preventing stale world-space steering
  during lag.
- [x] Decouple browser-player transmission from `onSensors`. Maintain one
  latest desired action, update it on pointer/button/boost changes, immediately
  request a bounded-rate send for meaningful changes (especially boost press
  and release), and periodically resend while ownership is active. The
  temporary implementation keeps both 30 Hz and 60 Hz selectable, replaces
  unsent old values rather than queueing them, and does not change the
  observation-driven Protocol 2 RL client into this browser timer.
- [ ] Measure the 30 Hz and 60 Hz candidates before selecting the configurable
  initial cadence. This remains a Stage 2 measurement, not an implied choice
  made by the temporary 60 Hz default candidate.
- [x] Keep these changes narrow and separable. Do not add another simulation,
  inference pool, client-side authority, or permanent TypeScript performance
  architecture.

**Correction tests, not golden-master preservation:**

- [x] A real nearby body changes nearest-body and hazard sensor values.
- [x] Exceeding the old collision capacity never makes segments disappear.
- [x] Record a correction fixture showing that reversing snake-array order must
  not change the intended collision result. It may remain red against the
  temporary TypeScript path; Stage 5 must make it pass in Rust.
- [x] Record a correction fixture requiring high-population spawning not to
  overlap complete bodies. Stage 5 owns the replacement rather than expanding
  Stage 1 into a TypeScript collision rewrite.
- [x] A disconnected player/RL snake never invokes neural control during the
  full grace window.
- [x] Assignment/reclaim/control/error traffic survives a saturated display
  path and a dropped first lifecycle send is visible/recoverable.
- [x] During induced catch-up, fresh player steering can affect the next
  eligible step and stale steering cannot persist indefinitely.
- [x] In a browser/server integration test, deliberately delay or suppress
  sensor messages while pointer and boost state continue changing. Prove fresh
  steering and boost release are still sent, accepted by the server, and can
  affect the next eligible fixed step. Repeat with display frames delayed or
  replaced.
- [x] Record the current external-join RNG contamination and the correction
  fixture; Stage 3 separates streams and Stages 5–6 make it pass.
- [x] Record the Float32 aliasing fixture now; Stage 6 supplies the checked v1
  mapping/range that makes it pass.

**Exit gate:** Active project records no longer bind future work to the
kernel-only mistake, and the temporary TypeScript reference observes bodies,
never silently drops collision entries, and keeps controller ownership and
critical messages correct enough to serve as evidence for the Rust port.
Browser-player steering and boost transmission no longer depends on receiving
a sensor message.

### Stage 2: Capture behavior, performance, archive, and database baselines

**Purpose:** Establish evidence before replacing subsystems. This stage
measures the production-shaped workload, not isolated shared-weight kernels.

**Behavior inventory and fixtures:**

- [x] Trace current `World`, `Snake`, `SimCore`, graph, controller, protocol,
  persistence, Hall-of-Fame, and browser save flows before writing their Rust
  replacements.
- [x] Capture the cross-cutting fixed-seed fixtures needed before the Rust
  spine: configuration normalization, graph ordering/offsets, RNG streams,
  sensor layout, one complete heterogeneous brain sequence, one world step,
  one generation boundary, frame v1, and checkpoint identity.
- [ ] Capture detailed movement/collision/evolution/command/import/resurrection
  fixtures immediately before the stage that ports each feature. Stage 2
  inventories their source paths and extraction method; it does not front-load
  an exhaustive fixture-writing project before Rust work can start.
- [x] Label each fixture `preserve` or `known defect—replace with stated
  rule`. Broken body sensing, truncating collision storage, array-order bias,
  unsafe overlap, immediate takeover, stale catch-up input, unreliable
  lifecycle traffic, sensor-gated browser action transmission,
  connection-driven RNG draws, and Float32 ID aliasing are correction
  fixtures, not golden masters.
- [x] Reproduce the three retained Git-history claims separately from
  current-source findings: introduction of the false owner-history documents,
  commit `3989d26`'s ten-/twenty-tick controller behavior, and the reported
  contents/defects of commit `8330065`. Preserve repository identity, exact
  commands, commit IDs, diffs, relevant file contents, and raw output in the
  factual implementation record. If a claim does not reproduce, correct the
  plan rather than repeating it.
- [x] Demonstrate current kill-credit behavior with head-to-body,
  simultaneous head-head, already-dead body owner, multiple candidate bodies
  and exact-tie examples before finalizing the selected working credit rule.
- [ ] Inventory real graph/preset IDs and record the current Windows and target
  Debian ordering, node offsets, Concat input order, parameter count, and
  architecture key. Retain the exact graph/save fixture, Node/OS/locale
  versions, commands, and both raw outputs; do not claim a real cross-OS
  difference unless the artifacts show one.

**Production-shaped performance baselines:**

- [x] Build one repeatable runner that records seed, graph, settings, process
  mode, worker count, display mode, controller mode, duration, and raw results.
- [ ] Measure default 55-snake play, a many-snake case, a large-brain case,
  dense long bodies/pellets, player control over LAN, the existing Protocol 2
  trainer, a long generation, and an accelerated soak.
- [ ] Record simulated seconds per real second, generation wall duration,
  fixed-step p50/p95/p99/max, subsystem time, Node event-loop delay, control
  and lifecycle latency, frame bytes/rate, collision-index load, CPU, RSS,
  Rust allocations where measurable, and dropped/capped wall debt.
- [x] Measure the real current count-one N-API production path separately from
  homogeneous kernel demonstrations, including crossings per controller
  update and full server-process throughput.
- [ ] Preserve raw benchmark output and environment details for the Ryzen 7
  2700 Debian VM with its eight available threads and 16 GiB RAM.
- [ ] Treat P0/P1/P2 target feasibility as unproven until these VM results
  exist. If a mandatory target later remains unreachable after profiling and
  reasonable optimization, report the limiting subsystem and measured capacity
  without weakening physics, collisions, sensors, or configured workload.

**Persistence and save-growth baseline:**

- [x] Inventory every current database table/column that grows per generation,
  including parent snapshots, per-slot weights, Hall-of-Fame genomes, fitness
  history, journals/WAL, and any repeated graph/settings data.
- [x] Inventory every current and legacy export representation and identify
  which existing user files must remain importable.
- [ ] Inventory the owner's real existing database copies and save files before
  narrowing or retiring any compatibility reader; record format, size,
  provenance and whether it can support exact or only compatible restoration.
- [ ] Measure raw packed Float32, decimal JSON, current legacy gzip, per-genome
  compression, raw-versus-shuffled-Zstandard adaptive payload choice, and
  whole-checkpoint compression on fresh/evolved default and large-brain
  populations.
- [ ] Reproduce every material prior numerical observation with named fixtures
  and retain the database/save/input bytes, generator or extraction script,
  command, codec/runtime versions, raw output, and environment. This includes
  the reported 5,921,520/6,041,600-byte database case, 7.68/14.65-percent
  Zstandard cases, approximately 20.03 JSON bytes per Float32, any existing
  save format/size, browser-memory peak, high-speed tunnelling threshold, and
  Ryzen/VM throughput. A failed reproduction updates the plan's dependent
  estimate or design assumption.
- [x] If empirical confirmation remains useful, run only a narrow disposable
  SQLite BLOB/chunk byte-volume experiment using representative compressed
  payloads. Record bytes written, transaction latency, WAL amplification,
  read-back throughput, deletion/page reuse, and main-event-loop impact if the
  synchronous experiment is intentionally run in the wrong place. Do not
  build a checkpoint schema, chunk checkpoint writer/reader, backup, pruning/
  GC, crash-recovery, export, or equivalent P0/P2/P3 persistence system.
  Complete checkpoint/backup/prune/recovery/interruption/overnight tests apply
  only to managed files.
- [x] Measure the write-validation choices on named P0/P2 managed checkpoints:
  single-pass publication, optional Zstandard frame checksum, lightweight
  container/entry scan, and full decode limited to manual export/pin/milestone
  classes. Record latency, I/O, diagnosis value, and choose the simplest policy
  that meets the checkpoint gate without restoring redundant digest layers.
- [ ] Measure archive encode/decode throughput and peak memory, including the
  Ryzen 7 2700 target rather than only the development machine.
- [x] Measure current browser heap and responsiveness during export/import,
  demonstrating the complete-response parse/stringify/Blob and
  read-as-text/parse/stringify duplication. Retain automation version, fixture
  files, raw memory traces, browser/OS environment, and screenshots/logs needed
  to reproduce the reported peak.
- [x] Produce deterministic accelerated fixtures representing at least an
  overnight number of checkpoints, histories, and Hall-of-Fame entries.
- [ ] Record database logical bytes, file bytes, WAL bytes, free pages,
  deletion latency, page reuse, checkpoint latency, restore latency, and
  compact/vacuum behavior.
- [ ] Use these measurements to set configurable archive/decoded/RSS/temp/free-
  disk limits and validate the owner-selected retention defaults. Report any
  proposed material storage-design or retention change for review rather than
  silently changing it.

**Performance checkpoint:** Publish a baseline report with no claim that
isolated shared-weight kernel timing validates the game. The report must make
the current slow-motion, control, browser-memory, and database-growth failures
reproducible and distinguish current-source structure, retained Git-history
evidence, prior planning observations, derived estimates, and newly reproduced
measurements.

**Exit gate:** The migration has a trustworthy behavior corpus and measured
starting point for game speed, control latency, browser/server memory, archive
size, and overnight database/managed-store growth.

### Stage 3: Build the minimum persistent Rust engine spine

**Purpose:** Establish only the non-throwaway Rust state, coarse Node boundary,
graph/RNG contracts, and checkpoint-v3 spine that Stages 4–6A require. Stage 4
must not wait for a complete persistence platform.

**Minimum engine and bridge:**

- [ ] Refactor the native crate into an engine library with a narrow N-API
  wrapper. Node sends validated coarse commands and routes bounded events; it
  does not step or inspect a copied world.
- [ ] Add a source-derived engine build/ABI/contract identifier and required-
  export handshake. A stale/mismatched addon fails before creating a world.
- [ ] Create one background coordinator thread, a bounded inbound queue,
  priority-aware bounded output, orderly start/stop/fault reporting, and no
  busy polling. Keep the message surface limited to initialization, commands,
  completed outputs, health/fault, and checkpoint descriptors.
- [ ] Prevent Rust unwinding across N-API/thread roots. A caught panic faults
  the experimental engine; broad hang recovery and supervisor/watchdog
  machinery remain Stage 6B/7 work.
- [ ] Define one deterministic scalar calculation interface and reusable
  scratch that Stage 7 can parallelize without changing semantics.
- [ ] Keep normal TypeScript startup unchanged and expose Rust only through an
  explicit experimental path.

**Persistent state, graph and RNG:**

- [ ] Represent normalized configuration, run identity, seed, allocator state,
  graph definition, population slots, brain handles, per-brain weights and
  recurrent state, world entities, controller leases and minimal generation
  state in Rust-owned types.
- [ ] Port seed normalization, labelled RNG streams, xorshift behavior,
  Gaussian spare state, serialization and restore. External connection
  bookkeeping consumes no world/evolution RNG.
- [ ] Port graph validation/compilation: raw-UTF-8 deterministic ordering,
  ports, Split/Concat order, outputs, parameter counts, architecture key,
  layout digest and offsets.
- [ ] Reject impossible dimensions/count arithmetic and unsafe requested state
  memory before publishing a partial engine.

**Non-throwaway checkpoint-v3 contract:**

- [ ] Implement the selected managed checkpoint-file adapter, not
  `snapshot_archive_chunks`: immutable digest-derived filenames plus small
  SQLite metadata/current-pointer/history/reference rows.
- [ ] Implement only the exact generation-boundary state needed for fresh
  experimental startup, generation transition and restart: identity, graph,
  settings, RNG/allocator, population metadata, packed weights and required
  recurrent/reset state.
- [ ] Implement adaptive `raw-f32le-v1` versus
  `f32le-shuffle4-zstd-v1` numeric encoding with bit-exact round trip, bounded
  scratch, logical per-role hashes and one logical root. Do not implement the
  full standalone archive here.
- [ ] Use the Stage 2-selected minimum write-validation policy. The provisional
  automatic-checkpoint path hashes/counts while writing, completes and fsyncs
  the codec/container, checks final length, and publishes once without a
  mandatory second full decode. Do not add redundant digest layers; record
  whether an optional frame checksum or lightweight scan was selected.
- [ ] Publish file-before-metadata using temp write, fsync, atomic rename and
  one small `synchronous=FULL` SQLite current-pointer transaction. Prove the
  minimum invariant that SQLite never points to a partial new file and Rust
  never announces an uncommitted checkpoint.
- [ ] Add the minimum persistence-worker/descriptor handoff needed to keep
  synchronous SQLite work off the main Node event loop. Population bytes never
  cross Node worker structured cloning.
- [ ] Round-trip one small and one representative checkpoint on Windows and
  Debian builds, including graph/RNG/weight-bit equality and next-step
  continuation.

**Explicitly deferred from Stage 3:**

- full standalone Export/Import archive composition and browser routes;
- automatic retention pruning, reference garbage collection and orphan
  sweeping beyond safe startup handling of the current operation;
- Hall-of-Fame compaction/pruning and complete history migration;
- broad current-v2, `genomes_blob`, population-JSON and older-file conversion;
- database vacuum/legacy compaction and hot-backup implementation;
- exhaustive malformed-archive/decompression-bomb campaigns;
- the complete commit/reply/swap/process-death failpoint matrix;
- multi-operation priority scheduling, advanced heartbeats, hang injection and
  watchdog/supervisor recovery;
- secondary commands and optional archive behavior.

Those items belong in Stage 6B or Track 7D unless Stage 4–6A exposes a specific
dependency. Moving one earlier requires naming that dependency; “persistence
should be complete first” is not sufficient.

**Foundation evidence gate:**

- [ ] Wrong-addon handshake, invalid graph/count/memory requests and RNG/layout
  fixtures fail deterministically before authority.
- [ ] Managed checkpoint write/read and file-before-pointer crash fixtures pass
  without retention, legacy conversion or exhaustive fault infrastructure.
- [ ] Run the coarse Node↔Rust bridge for ten idle minutes with no busy-poll
  growth; record queue depth, wakeups, latency, event-loop delay and memory.
- [ ] Static/runtime tracing proves there is no N-API crossing per snake, graph
  node, neural layer or fixed-step substage.

**Exit gate:** Rust owns the persistent state skeleton, deterministic
graph/RNG/population contracts, coarse responsive bridge and one durable
checkpoint-v3 path. Stage 4 begins immediately; standalone archives,
retention, legacy conversion, compaction and exhaustive durability work remain
explicitly unfinished.

### Stage 4: Port corrected sensing and complete heterogeneous population inference

**Purpose:** Move the first major hot pipeline into Rust and prove that Rust
evaluates the workload the application actually has: different observations,
weights, and recurrent state for each snake.

**World data needed for observation:**

- [ ] Implement packed snake/body/pellet storage with stable integer IDs,
  population slots, bounded brain handles, deterministic reuse, and checked
  growth/destruction.
- [ ] Implement pellet and body spatial indexes with complete cell coverage,
  deterministic duplicate suppression, checked capacity, reusable scratch,
  and no silent segment loss.
- [ ] Add initialization memory estimates for population, graph, recurrent
  state, bodies, pellets, indexes, scratch, frames, queues, archive staging,
  and the simultaneously live old/new states needed for import.

**Correct v3 sensing:**

- [ ] Port all 19 scalar fields, food bins, corrected body hazards, walls,
  other heads, nearest-food/body/head values, size-dependent ranges,
  cap/saturation diagnostics, and deterministic candidate ordering.
- [ ] Distinguish pure strategy probes from delivered samples and preserve the
  exact accumulated points-delta boundary.
- [ ] Initialize/reset `bestPointsThisGen` before any sensor pass after
  construction, generation transition, Reset, or import.
- [ ] Generate the sensor specification from Rust layout data checked against
  the current browser/RL contract.

**Complete graph inference:**

- [ ] Port Input, Split, Concat, Dense, activations, MLP, GRU, LSTM, RRU, and
  output mapping with reference scalar implementations.
- [ ] Evaluate the entire due heterogeneous population inside one Rust
  operation. Node must not be crossed between snakes, nodes, layers, or
  recurrent updates.
- [ ] Keep packed weights and recurrent state attached to stable brain handles;
  shuffled/shrinking due lists and resurrected/external brains cannot borrow a
  population slot's state.
- [ ] Reuse graph/query scratch and enable focused activation capture only on
  request.
- [ ] Use existing SIMD only behind scalar parity and runtime CPU detection.

**Correctness tests:**

- [ ] Corrected TypeScript and Rust sensor vectors match for preserve fixtures;
  known defects match the stated corrected result.
- [ ] Dense bodies/pellets, long segments, cap boundaries, headings, wall
  positions, sizes, and multiple bubble-bin counts are covered.
- [ ] Node-level, whole-graph, multi-step recurrent, reset, population
  replacement, shuffled due order, resurrected brain, and malformed-state
  tests pass within explicit tolerances.
- [ ] At least 55 deterministic genomes with distinct weights/state produce
  the expected distinct outputs.

**Performance checkpoint:**

- [ ] Benchmark sensing alone under default, many-snake, dense-body, and
  dense-pellet fixtures, recording p95/p99, candidate counts, index load,
  allocations, CPU, and memory.
- [ ] Benchmark complete heterogeneous inference for the default 55 snakes and
  large brains with different weights and recurrent state.
- [ ] Compare the scalar Rust population operation, SIMD where applicable, the
  current TypeScript graph path, and the count-one N-API path. A shared-weight
  batch is diagnostic only and cannot satisfy this gate.
- [ ] If either subsystem misses its interim budget, profile and correct it
  before building more hot work on top.

**Exit gate:** Rust produces correct real observations and complete
heterogeneous controller outputs without TypeScript or N-API work inside the
per-snake/per-layer loop.

### Stage 5: Port the complete world step, controllers, and scheduler

**Purpose:** Make Rust capable of advancing a correct authoritative fixed step,
including every hot system that shares world state.

**Movement and world interaction:**

- [ ] Port steering limits, heading update, speed, boost cost/eligibility,
  movement integration, body-point insertion/removal, length/radius,
  starvation, scoring, food consumption, growth, pellet respawn, death, and
  pellet drops.
- [ ] Preserve the one-observation-boundary rule: sample all due controllers
  from one stable pre-movement state, then move/resolve without resampling
  partial results.
- [ ] Use checked packed storage and reusable scratch; no per-snake/per-step
  heap allocation is accepted without measurement.

**Collision and spawning correctness:**

- [ ] Build one complete immutable collision read state, calculate all
  collision outcomes against it, and commit deaths/drops afterward in
  deterministic stable-ID order.
- [ ] Store all covered segments or fail the step before commit. Never continue
  with a truncated grid.
- [ ] Implement simultaneous head-head death for both, nearest-head pellet
  award with stable-ID ties, and deterministic wall/head-body/multi-snake
  handling independent of array order. Show current kill-credit examples
  before finalizing the working rule: body owner for head-to-body death and no
  kill for simultaneous head-head participants.
- [ ] Check spawn candidates against complete bodies and heads with a bounded
  fallback that either finds a valid placement or reports a clear admission
  failure; never knowingly start an overlapping population or silently reduce
  the configured count.
- [ ] Test configured maximum speed/body thickness for tunnelling and add
  swept/substep handling where required by the corrected rule.

**Controller ownership and lifecycle:**

- [ ] Port baseline bots, neural ownership, player leases and Protocol 2 RL
  leases. Hold accepted input for configurable 500 ms, then neutral/boost-off;
  reserve ownership for configurable 30 seconds with no brain; at expiry make
  one external-to-neural transition and invalidate the old controller.
- [ ] Give browser-player actions latest-value semantics: accept independent
  periodic/change-triggered sends, replace an older unconsumed action, and
  drain the newest accepted value before each eligible fixed step. Keep
  Protocol 2 RL actions observation-driven and ordered under their compatible
  contract rather than forcing them onto the browser cadence.
- [ ] External join/reclaim/disconnect bookkeeping must not draw from world or
  evolution RNG.
- [ ] Prioritize assignment, reclaim, input, sensor, errors, and lifecycle
  events over display; keep only the latest replaceable frame.
- [ ] Keep Protocol 2 JSON field compatibility for the first Rust path.
  Connection/assignment epochs reject stale input, while lifecycle priority
  and observable send failure prevent silent loss. New binary batching,
  lockstep modes, broad acknowledgement/replay systems, and coalescing
  protocols are not on the critical path.

**Fixed-step scheduler and commands:**

- [ ] Implement the documented complete step order, fixed delta, independent
  `simSpeed`, settings/God Mode/action drain before each step, and atomic
  pre-step setting application.
- [ ] Check commands before every overdue step, not only once per Node pump.
- [ ] Bound wall-debt horizon, publish overload diagnostics, and prevent long
  uninterrupted catch-up bursts. Interactive use requires a Node/socket
  service opportunity between overdue steps.
- [ ] Preserve command arrival order across Node batching and never publish a
  frame, stats result, checkpoint success, or lifecycle success for an
  uncommitted step.

**Correctness tests:**

- [ ] Movement, boost, food, growth, starvation, drops, and death match
  preserve fixtures.
- [ ] Reversing entity/container order cannot alter corrected collision
  results, and repeated high-population starts never overlap.
- [ ] Player/RL control always overrides neural control while owned; the
  500-ms hold then neutral behavior is wall-time exact; the 30-second grace
  prohibits brain takeover; expiry cannot produce a player/brain mixture; and
  reclaim rejects old-epoch actions.
- [ ] Baseline, neural, player, and RL observations share one environment
  boundary and points-delta semantics.
- [ ] A matching accepted Node send result advances the external
  points-delta boundary exactly once; failed, stale-epoch, duplicate, or
  replaced results do not advance it.
- [ ] Commands are observed between every catch-up step and fresh input can
  affect the next eligible step under induced overload.
- [ ] With sensor delivery delayed or suppressed, change browser pointer and
  boost state and prove the newest steering plus boost release still traverses
  the thin boundary and can affect the next eligible step. No queued backlog of
  obsolete browser actions may replay afterward.
- [ ] Fixed delta never grows with `simSpeed`; overload is visible rather than
  silent slow motion or physics skipping.

**Performance checkpoints:**

- [ ] After movement/body/pellet work, benchmark that subsystem with long
  bodies and dense pellets.
- [ ] After spatial/collision work, benchmark a complete collision phase with
  maximum admitted grid load and report p95/p99 plus capacity.
- [ ] After the scalar fixed step is complete, measure simulated seconds per
  wall second, fixed-step p95/p99, memory, allocation count, and subsystem
  shares for default, many-snake, large-brain, and dense-world fixtures.
- [ ] Run player and RL control under artificial slow steps and record action,
  sensor, assignment, and reclaim latency before any optional protocol work.
- [ ] Compare browser-player 30 Hz and 60 Hz resend candidates under P5,
  including sensor/display suppression, rate-limiter coalescing, CPU/network
  cost, change-to-send latency, and accept-to-step latency. Report the selected
  configurable cadence; do not apply it to the RL trainer.

**Exit gate:** A scalar Rust engine can advance the complete authoritative
world continuously with correct sensing, controls, physics, collisions, and
scheduling. Node is not the world clock or hot-loop orchestrator.

### Stage 6: Complete generations, frame v1, archives, managed retention, and thin-interface wiring

**Purpose:** Reach the earliest minimal complete round through the current
browser and RL contracts. This stage implements only what is needed for a
usable compatible Rust game and durable save/restore.

**Evolution and generation lifecycle:**

- [ ] Port fitness, statistics, stable tie rules, elites, tournaments,
  crossover, mutation, recurrent-specific operations, early-end behavior,
  generation duration, and generation transition ordering.
- [ ] Keep evolved population slots stable through a generation and keep
  baseline, external, and resurrected brains outside dense selection.
- [ ] Create the next population and exact resumable checkpoint at the
  documented zero-recurrent-state boundary before spawn/pellet/gameplay draws.
- [ ] Stage the generation replacement until its managed checkpoint file and
  required small metadata/current-pointer transaction succeed; a failed write
  leaves the prior world, RNG, identity, compact history, and pending
  generation event current.

**Current browser display and commands:**

- [ ] Pack the existing binary display-frame v1 directly in Rust and route it
  to the unchanged parser/renderer without reconstructing a TypeScript world.
- [ ] Double-buffer/reuse frame memory until socket send completes and keep at
  most the latest unsent visual frame.
- [ ] Emit the minimum current stats, welcome/assignment/lifecycle results,
  errors, and health through the thin event boundary.
- [ ] Expose the latest Rust-packed frame byte length as small routing/welcome
  metadata. Remove any welcome refresh that serializes or reconstructs the
  complete authoritative world merely to calculate that field, and test that
  repeated refreshes perform no extra world serialization.
- [ ] Keep exact-ID limits explicit for v1. A frame-v2 integer-ID/culling/LOD
  project is not a prerequisite unless measured v1 bytes prevent the required
  control-message latency.

**Stage 6A — first usable Rust vertical slice:**

This milestone is completed and run before the remaining Stage 6 archive,
retention, Hall-of-Fame, God Mode, resurrection, and compatibility work. Those
features must survive the migration, but they do not block the first real
browser/RL exercise of the Rust game.

- [ ] Start the Rust engine under an explicit experimental/fresh command with
  the default graph and one supported configuration.
- [ ] Node validates/routes WebSocket traffic, serves browser assets, and owns
  only the minimum run-start/generation checkpoint transaction through the
  dedicated persistence worker; the main event loop does not block on SQLite
  and Node does not inspect authoritative game arrays.
- [ ] Route Rust frame v1, basic stats, join/assignment, player steering/boost,
  Protocol 2 sensors/actions, death/reassignment, disconnect/reclaim, and
  lifecycle/errors through the existing browser and trainer contracts.
- [ ] Run the browser player's independent latest-action sender through the
  real LAN path: change-triggered bounded sends plus the measured periodic
  resend continue while sensors or display frames are delayed/suppressed.
  Direction is calculated at send time from screen pointer and newest
  camera/player state. The Protocol 2 trainer remains observation-driven.
- [ ] Preserve trusted-LAN host/UI/WebSocket configuration, CORS combinations,
  reconnect behavior, and health.
- [ ] Implement the final checkpoint-v3 writer/reader for exact
  run-start/generation boundaries and one protected current pointer: Rust
  creates and publishes one immutable managed file, and the persistence worker
  transactionally inserts its metadata/current pointer, one compact
  generation-history record, and required stable graph/config references. The
  experimental process must restart from the latest valid boundary; this is
  not a throwaway persistence format.
- [ ] During 6A only, preserve the current every-generation checkpoint cadence
  in a dedicated fresh experimental database and do not automatically delete
  any checkpoint. Bound the short experiment by free-disk admission. The
  owner-selected every-generation cadence and retention defaults are
  implemented in Stage 6B; external archives, legacy import, and secondary
  commands follow there as well.
- [ ] Keep the TypeScript game as a separately selected reference path. One
  process/run has exactly one authority and never silently falls back after a
  Rust fault.

**Stage 6A evidence gate:** The current browser can render and play one complete
Rust-owned generation over LAN and observe the transition into the next, while
the unchanged Protocol 2 trainer can assign, observe, act, die/reassign,
disconnect, and reclaim. Run
the scalar path through the actual Node process immediately and record
full-step p95/p99, simulated/wall ratio, browser/RL/lifecycle latency, frame
bytes, checkpoint pause, Node responsiveness, and RSS. Do not wait for Stage
6B or parallelism to learn whether the real vertical slice works. The gate
includes suppressed browser sensors and replaced display frames while fresh
steering and boost release still reach the next eligible fixed step.

**Stage 6B — direct adaptive archive export:**

- [ ] The browser's Export action immediately activates one ordinary
  attachment link/navigation. It does not fetch/parse the archive, poll a
  preparation token, reconstruct genomes, stringify the save, create a
  population Blob, or convert weights with `Array.from`.
- [ ] Bind that direct request to one exact durable checkpoint/root digest and
  keep that reference alive across concurrent generations, Reset/New Run, and
  pruning.
- [ ] Rust writes one archive to a same-directory `.partial` bounded temporary
  file while calculating logical counts/hashes and choosing raw versus
  shuffled-Zstandard per large payload. Finish/fsync, check final length,
  atomically rename to `.ready`, and fsync the parent directory where required
  before Node sends success headers. Apply the Stage 2-selected write-validation
  class policy: do not add a redundant full second decode to every automatic
  checkpoint, while permitting a measured manual-export/pin/milestone decode,
  cheap frame checksum, or lightweight scan if selected. Node sends
  `Content-Type`, `Content-Disposition`, and `Content-Length` and streams the
  opaque ready bytes with backpressure.
- [ ] Delete temporary export files after success, socket cancellation, or
  preparation/response failure. On startup, remove only recognized
  unreferenced stale temp files older than the documented grace period.
- [ ] Serialize direct export operations initially; reject a concurrent second
  request with a small busy response, make success/failure/cancellation
  terminal, and require a fresh direct request after a failed transfer.
  Enforce the reviewed server-work and active-transfer no-progress rules.
- [ ] Tiny browser-only preferences are stored separately or excluded unless
  existing behavior proves they are authoritative; the population never
  travels to the browser merely to attach them.

**Stage 6B — direct binary archive import:**

- [ ] The browser sends the selected original `File` directly as the raw body
  of a dedicated archive endpoint, using `XMLHttpRequest.send(file)` for
  browser-visible upload progress. V1 does not add a multipart parser. Browser
  code does not call `readAsText`, parse the population, reconstruct objects,
  or stringify a second request.
- [ ] Spool the archive upload to a bounded temporary file while enforcing its
  configured archive-byte limit. The generic 50 MiB JSON-body limit does not
  apply.
- [ ] Before constructing a replacement world, validate archive magic/version,
  safe USTAR roles, declared decoded bytes, entry lengths/counts, logical
  hashes/root, graph layout, calculated Rust working set, configured
  archive/decoded limits, free disk and managed-store/database capacity.
- [ ] Report archive bytes, declared decoded bytes, record counts, calculated
  state size, free disk and the applicable limit in plain language on
  rejection.
- [ ] Stream/decode into staged bounded Rust/database structures and commit
  only after complete validation. Clean temporary uploads after success,
  failure, cancellation, and documented startup scavenging.
- [ ] Return only small progress/result/error messages to the browser.
- [ ] Enforce the Stage 2-derived configurable archive/decoded/RSS/temp/
  no-progress envelope and prove that a progressing large upload has no short
  total-duration timeout.

**Stage 6B — managed-checkpoint retention, history, and remaining durability:**

- [ ] Reuse the Stage 3/6A managed checkpoint-v3 file reader/writer; extend its
  small metadata transaction with Hall-of-Fame references, checkpoint classes,
  pin state and retention metadata while preserving versioned history and
  graph/config references. Never introduce SQLite checkpoint chunks or hold a
  full checkpoint in Node/Rust bridge memory.
- [ ] Enforce the full transient-disk admission formula before checkpoint,
  import, export, and pin. Disk failure preserves the prior durable/current
  boundary and does not prune first.
- [ ] Classify the automatic managed checkpoint files already produced by the
  final writer and apply the owner-selected eight-recent, twelve-milestone/
  25-generation, two-prior-run and configurable 4 GiB policy in the crash-safe
  metadata-then-unlink workflow.
- [ ] Always retain the latest valid resumable checkpoint; never automatically
  delete pinned checkpoints or downloaded exports. Apply the selected best-50-
  unique-plus-pinned Hall-of-Fame policy only after existing-data migration is
  verified, and preserve inexpensive compact metadata for older entries.
- [ ] Store every per-generation eight-field summary in compact fixed-width
  records without downsampling. Keep the UI chart window separate from durable
  history.
- [ ] Store Hall-of-Fame large weights as adaptively raw/compressed packed
  binary in the
  content-addressed/deduplicated store, rather than one decimal JSON genome per
  generation.
- [ ] Atomically commit checkpoint/current pointer, one idempotent run/generation
  history row, Hall-of-Fame metadata, and all content-addressed references.
  Reference-aware garbage collection must not break any retained/pinned
  restore or export.
- [ ] Replace v2 `.all()` loading, legacy combined synchronous gunzip, and
  population-sized format-null/0 `payload_json` selection with the bounded
  row/BLOB/TEXT-slice streaming readers measured in Stage 2.
- [ ] Do not silently downsample or delete user-visible history. Any future
  high-volume diagnostic series requires a separate reviewed retention rule.
- [ ] Implement orphan scanning and metadata/file reference garbage
  collection; verify every retained/pinned checkpoint and Hall-of-Fame object
  before deleting an unreferenced managed file.
- [ ] Keep ordinary WAL/page maintenance bounded and observable. Leave the
  explicit full legacy-database compaction command and overnight-sized vacuum
  measurements to Track 7D.

**Stage 6B — remaining commands and browser surfaces:**

- [ ] Replace the vague Save concept with separate user-visible “Pin
  checkpoint” and “Export archive” operations. Neither endpoint/button
  silently performs both.
- [ ] Stage Reset, New Run, and import replacement until each required database
  transaction succeeds. Failure leaves the prior world, RNG, identity,
  settings, graph, history, Hall of Fame, and pending events current.
- [ ] On successful replacement, advance `worldEpoch`, invalidate every old
  assignment/action/observation/send-result/token, keep sockets open, emit one
  reliable state-replaced result, and place clients in `awaitingRejoin`. No
  fresh assignment exists before a new ordered join, so live sockets never
  carry stale control across the boundary.
- [ ] Route atomic settings, bounded chart windows, Hall-of-Fame events,
  focused activations, God Mode, Reset/New Run, import, resurrection, and their
  small results through the thin event boundary.
- [ ] Preserve the current user-visible behavior of each surface unless a
  recorded correction or recorded owner decision explicitly changes it.

**Stage 6B completeness tests:**

- [ ] A complete small generation, next-population weights, RNG continuation,
  history, Hall of Fame, frame v1, and lifecycle order match preserve/corrected
  fixtures.
- [ ] Current browser rendering, selection within v1 limits, player control,
  settings, God Mode, Reset, New Run, resurrection, and reconnect work against
  Rust frames/events.
- [ ] Repeated welcome-state refreshes read the latest packed-frame byte-length
  metadata and never trigger a second full-world serialization or allocation.
- [ ] Existing Protocol 2 trainer completes assignment, observations, actions,
  death/reassignment, disconnect, and reclaim against the Rust engine.
- [ ] With a player and RL controller connected, Reset, New Run, successful
  import, and failed import prove old actions/tokens cannot cross `worldEpoch`,
  lifecycle messages arrive in order, and the selected rejoin/reassignment rule
  is compatible with both clients.
- [ ] Copied current/legacy databases restore through bounded
  row/BLOB/TEXT-slice readers; supported current/legacy files restore through
  bounded server-side upload readers. Do not describe database-only formats as
  browser-uploadable files.
- [ ] Export/import round trips settings, graph/layout, population weights,
  recurrent state where applicable, RNG, allocators, history, Hall of Fame,
  run identity, generation state, and exact generation-boundary continuation
  semantics. Hall of Fame remains run-scoped and an older conflicting exact
  boundary creates an explicit provenance-labelled branch.

**Stage 6B performance checkpoint:** Repeat the Stage 6A integrated run with
archive creation/import, retention, history, Hall of Fame, and remaining
commands active. Record archive size/time, import peak, checkpoint/maintenance
pause, database/WAL bytes, and any regression in step/control latency before
adding parallelism or optional protocols.

**Exit gate:** An explicitly selected experimental Rust engine can complete
rounds, render in the current browser, serve the current RL trainer, and
save/restore through bounded adaptive archives, managed retained checkpoint
files and a small metadata/history database.

### Stage 7: Prove and optimize the experimental Rust-authoritative program on the real workload

**Purpose:** Exercise the complete experimental path end to end, fix
compatibility failures, add only measured parallelism, and prove the supported
VM/LAN/save envelope before changing normal startup.

Stage 7 has two independent tracks. **7P performance** begins immediately when
the Stage 6A playable vertical slice passes; it does not wait for archive,
legacy, Hall-of-Fame, God Mode, or retention completion. **7D durability**
continues Stage 6B archive/database work and A1–A10 in parallel. Stage 8
production cutover requires both tracks, but neither delays the other merely
because secondary durability work is unfinished.

**End-to-end compatibility and failure work:**

- [ ] Run all browser, Protocol 2, HTTP, persistence, startup/resume, LAN,
  controller, collision, evolution, and fault paths with no live TypeScript
  `World`.
- [ ] Verify caught Rust panics/returned faults preserve `/health`, reject the
  in-flight authoritative step, publish no partial frame/stats/checkpoint
  success, and never trigger a TypeScript fallback.
- [ ] Separately inject process abort/kill during steps, checkpoint, export,
  import, and pruning; verify supervised restart, client reconnect, temp
  cleanup, and recovery from only the latest fully committed checkpoint without
  claiming `/health` survived the dead process.
- [ ] Test current laptop and desktop browsers over trusted LAN and the
  separate desktop RL trainer against the Debian-hosted experimental process.
- [ ] In P5, suppress/delay browser sensors and replace/delay display frames
  while pointer and boost state change. Prove immediate bounded change sends
  and periodic latest-value resends leave the browser, reach Node/Rust, and can
  affect the next eligible step; boost release cannot remain trapped in the
  browser. Verify the RL trainer still produces actions from observations
  rather than inheriting the player timer.

**Track 7D — end-to-end archive and database acceptance:**

- [ ] Export a population substantially larger than the former 50 MiB JSON
  limit—including a fixture whose actual archive file exceeds 50 MiB—as one
  ordinary download and import the exact file by direct upload.
- [ ] Instrument browser JavaScript heap during both operations and prove it
  does not scale with decompressed population size; the tab remains responsive
  and no population-sized string/object/Blob exists.
- [ ] Measure server/Rust peak memory during archive construction, file
  download, upload spooling, validation, decompression, staged state
  construction, managed-checkpoint publication, small metadata commit and
  state swap.
- [ ] While each checkpoint/prune/export/import/legacy-conversion/WAL/vacuum
  operation runs, measure Node event-loop delay and player/RL/control/health
  latency; synchronous persistence work must remain confined to its worker.
- [ ] Record raw and shuffled-Zstandard candidate sizes, selected per-payload
  encoding, container overhead and decimal-JSON ratios for default/evolved/
  large-brain fixtures; never fail a valid high-entropy raw selection merely
  for missing an arbitrary compression percentage.
- [ ] Record the actual cost and diagnostic value of the selected write-time
  validation policy for automatic, milestone, pinned, and manual-export
  classes. Corrupt a file after successful single-pass creation and prove
  restore discovers it, latest-start recovery branches from the previous valid
  retained checkpoint, and the corrupt evidence remains preserved. Do not add
  redundant hash layers or universal second decodes without a measured,
  distinct failure they prevent.
- [ ] Prove round-trip preservation of every required state field and exact
  version-scoped continuation semantics.
- [ ] Prove truncated, corrupt, unsupported, false-size, bomb, incompatible
  graph-order, interrupted/lying/chunked-over-limit upload, disk/temp/WAL/memory
  admission failure, connected-no-progress transfer, concurrent busy job,
  encoder failure, restart, and cancelled download paths leave
  the current run and database valid and clean their staged/temp artifacts.
- [ ] Exercise FULL-commit failpoints before commit, after commit/before reply,
  after reply/before Rust swap, and after swap/before public success. Also hang
  calculation/archive/persistence jobs and prove watchdog-supervised recovery.
- [ ] Prove an export remains bound to its selected checkpoint while the game,
  checkpointing, and pruning continue; validate `.partial` to `.ready`
  atomicity, directory durability, two simultaneous direct requests, terminal
  cancel/failure plus a fresh request, one completed response, and stale
  cleanup.
- [ ] Repeat identical import, same-key/different-root conflict, and older
  same-run import with future generations. Prove idempotent reuse, atomic
  rejection, or the explicitly selected non-destructive branch behavior
  without row overwrite/deletion.
- [ ] Start from copied databases containing supported v2 rows, legacy
  `genomes_blob`, and format-null/0 `payload_json`, and directly upload
  supported current/older save files. Keep those compatibility entry paths,
  provenance guarantees, and memory measurements distinct.
- [ ] Add static/integration checks forbidding population-wide
  `response.json()`, `response.text()`, `FileReader.readAsText()`,
  `.arrayBuffer()`, `Response.body`/`getReader()`, `File.stream()`, manual
  chunk accumulation, `JSON.parse()`, `JSON.stringify()`,
  `Array.from(weights)`, and browser-created population `Blob` operations in
  the normal save path. Positively assert the direct-link/direct-File call
  graph.
- [ ] Run a real overnight soak or deterministic accelerated equivalent with
  at least the same generations/data volume; verify pruning, pinned
  protection, compact history, Hall-of-Fame storage, managed-file reclamation,
  orphan cleanup, small-WAL behavior, maintenance pauses and the configured
  disk budget. Restore and independently validate every retained
  current/prior/pinned anchor after pruning.
- [ ] Validate cold and hot database-plus-managed-directory backups, including
  a backup racing pruning, a missing referenced file, a harmless orphan and
  recovery from copied backup sets. Never document `slither.db` alone as a
  complete checkpoint-v3 backup.
- [ ] Measure the explicit offline full-compaction path only on copied legacy
  databases large enough to represent the prior overnight failure.

**Track 7P — measured bounded parallelism:**

- [ ] Profile the complete scalar engine before parallelizing.
- [ ] Activate one persistent bounded calculation-worker pool using the scalar
  job/result contract.
- [ ] Give each worker isolated graph, sensor-query, movement-proposal, and
  collision-detection scratch.
- [ ] Parallelize due sensing plus complete brain evaluation by stable handle,
  disjoint movement proposals where safe, spatial-index building through
  deterministic merge, and collision detection but not collision commit.
- [ ] Preserve stable merge/commit order so worker count cannot change deaths,
  outputs, recurrent ownership, RNG draws, events, or the next population.
- [ ] Reuse hot buffers and remove measured allocation sources before adding
  specialized SIMD/affinity complexity.
- [ ] Compare four, five, six, and—only where useful—seven calculation workers
  on the assigned eight-thread Ryzen 7 2700 while reserving capacity for Node,
  the persistence worker, coordinator, single archive worker, networking,
  runtime helper threads, SQLite, and the OS. Repeat with checkpoint/export
  activity so an idle-only winner is not selected.

**Required workload gates:**

- [ ] Default 55 snakes at 1x: a configured 60-second round finishes in
  approximately 60 real seconds after warm-up with no dropped wall debt.
- [ ] The agreed many-snake and large-brain cases meet their selected
  simulated/wall target and memory ceiling.
- [ ] Dense long bodies/pellets produce no false-clear sensors, truncated
  collision entries, order-dependent deaths, overlap piles, or phasing.
- [ ] Browser play over LAN remains responsive through display loss and
  reconnect; sensor loss cannot trap fresh steering or boost release in the
  browser, and neural control never fights the player during ownership/grace.
- [ ] Existing RL training remains usable at its agreed observation/control
  rate without lost assignment or hidden transition gaps.
- [ ] Record fixed-step p95/p99, subsystem timing, input/control/lifecycle
  latency, event-loop delay, CPU, RSS/peak, frame bytes, collision load,
  database/archive growth, and soak stability for each supported workload.
- [ ] If P0, P1, or P2 remains below its mandatory target after proper
  profiling and reasonable optimization, publish the raw result, limiting
  subsystem, and measured capacity and return the supported-envelope issue for
  owner review. Do not manufacture a pass by weakening physics, collisions,
  sensors, or the configured workload; production cutover remains blocked.
- [ ] One-worker and every accepted production worker count produce the same
  discrete results and tolerance-bounded numeric results.

**Exit gate:** The explicit experimental path is a complete, usable
Rust-authoritative program on the real Debian VM, browser LAN clients, and RL
trainer, and it meets the selected performance, memory, persistence, and
correctness envelope.

### Stage 8: Cut over production, retire the obsolete game, and defer optional protocols

**Purpose:** Make the proven Rust engine the normal program, remove redundant
hot paths only after that cutover, and keep speculative protocol work from
blocking delivery.

**Production cutover:**

- [ ] Make the Rust engine the normal required startup path.
- [ ] Refuse fresh and resumed production startup when the loaded addon's
  source-derived build/ABI/contract identifier or required exports do not
  match the Node interface; include a stale-addon negative deployment test.
- [ ] Stop constructing TypeScript `SimCore`, `World`, `Snake`, `GraphBrain`,
  `BrainPool`, and inference workers in production.
- [ ] Remove ordinary `--backend js`/old Node-MT production modes; retain a
  clearly selected test/reference runner through migration and stabilization.
- [ ] Keep Node limited to LAN HTTP/WebSocket/static routing, archive byte
  transfer, managed-file descriptors, SQLite metadata/history, health, and
  small event translation.
- [ ] Never silently fall back to the TypeScript game after Rust startup or
  runtime failure.
- [ ] Update status output to report Rust build, worker count, overload,
  archive/checkpoint-store/database status, and achieved simulation rate
  honestly.
- [ ] Run fresh, latest-resume, explicit-resume, Reset, New Run, import,
  export, pinned save, Hall of Fame, resurrection, player, and RL acceptance
  once more on the production command.

**Retirement and deployment:**

- [ ] Prove by import/dependency search that production reaches no TypeScript
  game loop, old per-layer bridge, `server/brainPool.ts`, or
  `server/worker/inferWorker.ts`.
- [ ] Preserve useful TypeScript fixtures/reference code in an explicit test
  location through migration and stabilization. Production imports are
  removed regardless. After Rust remains stable, active reference code may be
  archived or removed, but destructive deletion is neither required nor a
  cutover task.
- [ ] Remove obsolete worker protocols, hot-path copies, configuration, and
  dependencies; retain Rust SIMD helpers only when used and measured.
- [ ] Build the release addon on/for Debian x86_64 GNU, build browser assets
  once, serve without Vite development overhead, and supply plain-language
  start/stop/update/backup/restore/maintenance instructions.
- [ ] Install/test the Debian systemd unit with bounded `Restart=on-failure`;
  inject a caught engine panic, process abort, and hung-worker watchdog exit,
  and prove restart selects a valid committed checkpoint without a restart
  loop or TypeScript fallback.
- [ ] Verify `0.0.0.0` or explicit LAN binding, laptop/desktop browser access,
  desktop trainer access, service startup, logs, database-plus-managed-file
  backups, and portable export backups.
- [ ] Publish the measured supported workload, overload behavior, controller
  grace/reclaim behavior, archive compatibility/limits, retention policy,
  expected database/managed-store footprint, and recovery instructions.

**Post-cutover work, explicitly not a prerequisite for first production Rust:**

- [ ] Measure whether frame v1 bytes or Float32 ID limits now justify a
  negotiated integer-ID frame v2, viewport culling, distant-body detail
  reduction, or a separate display transport. Implement only with browser
  parser/render/selection changes and compatibility tests.
- [ ] Coordinate with the separate trainer project before adding binary
  observations/actions, population batching, lockstep, coalesced observations,
  explicit gap accounting, or other Protocol 2 successors.
- [ ] Add richer lifecycle acknowledgement/replay only where measured failure
  modes remain after critical-message priority, latest-frame replacement, and
  minimal assignment epochs.
- [ ] Treat these items as separately reviewed follow-up work. Their absence
  cannot be used to delay or reject the compatible Rust cutover unless a
  measured supported-workload gate proves one is immediately necessary.

**Final acceptance:**

- [ ] Normal startup has exactly one authoritative Rust game and no production
  TypeScript simulation.
- [ ] A 60-second supported 1x round finishes in approximately 60 real seconds,
  remains playable over LAN, and serves the existing RL trainer.
- [ ] The normal browser performs archive download/upload without
  population-sized JavaScript work, and overnight operation remains inside the
  selected database budget.
- [ ] Dense collision/sensing and controller regression suites pass with no
  false-clear bodies, dropped segments, overlap piles, phasing, neural/player
  conflict, lost lifecycle messages, or silent slow motion.
- [ ] A copied compatible database and supported legacy/current save files can
  be restored on the Debian VM.

**Exit gate:** The owner can run, play, train, save, restore, and leave the
program operating overnight on the intended server with Rust owning the game,
Node acting as a thin interface, and the browser doing only rendering,
controls, and ordinary file transfer.

## Feature acceptance matrix

This prevents the migration from becoming “fast” by quietly dropping features.

| Feature | Required proof before cutover |
|---|---|
| Browser spectator | Remote LAN browser receives current Rust frames/stats and can follow/select. |
| Browser player | Join, steer, boost, die/reassign, disconnect/reclaim, and visual response pass under P0/P1/P2; sensor/display suppression cannot prevent fresh steering or boost release from reaching the next eligible step. |
| RL trainer | Existing Protocol 2 JSON bot handshake, sensors, actions, death replacement, reconnect, and configured accelerated run pass from the real desktop trainer. |
| Trusted LAN | Wildcard/explicit binds, URL derivation, CORS, launcher output, and Vite dev HMR work from another device. |
| Default graph | MLP-GRU-Dense weights/output/state pass fixtures and integrated generation. |
| Custom graphs | Dense/MLP/GRU/LSTM/RRU/Split/Concat validation, persistence, inference, and viz pass. |
| Sensor v3 | Every label, corrected body sensing, delivery delta, baseline/player/RL consistency pass. |
| Snake physics | Turn/speed/boost/body/food/growth/wall/death fixtures pass. |
| Collision | No overflow, order invariance, swept crossing, head-head rule, safe spawn, stress pass. |
| Baseline bots | Strategy/life-stage behavior, body avoidance, stable seed, respawn pass. |
| Neuroevolution | Fitness, elite/tournament/crossover/mutation, history, fair collision selection, next generation pass. |
| Simulation time | Fixed delta and P0/P1/P2 wall-time ratio/generation duration pass. |
| Live settings | Atomic boundary application and browser authoritative response pass. |
| Reset-only settings/graph | Rebuild and durable boundary pass. |
| Reset | Same seed, new run ID, generation one, durable before acknowledgement. |
| New Run | New entropy seed/run ID and durability pass. |
| Save/resume | Packed adaptive v3 exact-boundary round trip, startup selection, corruption rejection, and crash-safe managed-file/current-pointer transition pass. |
| Legacy data | Copied databases with v2 rows, bounded `genomes_blob`, and format-null/0 `payload_json` migrate through startup/resume readers; current JSON and identified older compressed files migrate non-destructively through direct upload, with honest provenance/completeness rather than invented exact resume. |
| Export/import | All A1–A10 direct-download/direct-upload, bounded-memory, compression, atomicity, legacy, and browser-usability gates pass. |
| Retention/database | Owner-selected recent/milestone/prior-run/pinned policy, configurable byte cap, managed-file reclamation, small WAL, overnight fixture, and no pinned/export deletion pass. |
| History | Complete compact per-generation summaries survive pruning/export/import without repeated population data or unapproved downsampling. |
| Hall of Fame | Correct best genome/event flow, packed deduplicated weights, owner-selected retention, archive round trip, and browser behavior pass. |
| Resurrection | Compatible genome, safe ID/spawn, collision/frame behavior pass. |
| God Mode | Kill normal death path, whole-body bounded move, reliable result pass. |
| Neural visualization | Focused Rust activations work without all-population overhead. |
| Frames/rendering | Rust-produced v1 compatibility, newest-frame replacement, lifecycle priority, cached welcome byte-length metadata without extra world serialization, and browser budgets pass; a later format is not a cutover gate unless measurement promotes it. |
| Failure handling | Rust worker/coordinator fault, queue saturation, DB failure, invalid state expose honest status and no partial success. |
| Shutdown | SIGTERM stops cleanly and preserves only valid durable boundaries. |

## Defect-to-stage tracking

| Defect | Primary repair stage | Final proof |
|---|---:|---|
| GOV-001 false owner history | 1 | Active-document search and owner review |
| ARCH-001 wrong Rust boundary | 3–8 | Normal startup has no TS World/per-layer crossing |
| INF-001 count-one production batches | 4 | Heterogeneous full-graph population benchmark |
| THR-001 unused VM threads | 7–8 | Active Rust workers and P0–P5 VM results |
| THR-002 Node worker overhead | 6–8 | Node inference pool absent from production imports |
| SCH-001 overload spiral | 1, 5 | Interim yield plus Rust action-between-steps regression |
| TIME-001 multi-minute rounds | 2, 5–8 | VM 60-second generation wall-time gate |
| CTRL-001 immediate takeover | 1, 5 | Disconnect/reclaim/grace integration |
| CTRL-002 old tick timeout | 1, 5 | 0.1x/1x/12x wall-time grace tests |
| CTRL-003 lost assignment | 1, 6–7 | Lifecycle priority under a full display socket |
| CTRL-004 stale input through batch | 1, 5–7 | Command drain between fixed steps under induced load |
| CTRL-005 stale mouse target | 1, 6 | Moving-camera stationary-pointer test |
| CTRL-006 sensor-gated browser send | 1, 5–7 | Suppressed-sensor pointer/boost integration through next eligible step |
| SENSE-001 body blindness | 1, 4–5 | Real-grid neural/baseline/RL tests |
| COLL-001 silent capacity loss | 1, 5 | >200k segment stress/no omission |
| COLL-002 order-biased deaths | 5 | Reverse-order invariant outcome |
| COLL-003 overlapping spawn | 5 | High-population separation fixture |
| COLL-004 range/tunnelling | 5 | Config and swept-crossing fixtures |
| RNG-001 external join contamination | 3, 5–6 | Join/reclaim RNG-continuation test |
| OBS-001 split observation boundary | 2, 4–5 | Same-state external/neural/baseline fixture |
| GRAPH-001 locale-dependent weight order | 2–4, 7 | Cross-OS manifests and non-destructive legacy map |
| FRAME-001 client overload | 2, 6–8 | P1/P4 LAN/browser budgets; later format only if needed |
| FRAME-002 welcome full-world serialization | 6 | Welcome refresh reads cached packed-frame length with zero extra world serialization |
| ID-001 Float32 resurrected-ID aliasing | 5–6 | Checked v1 mapping and multi-resurrection test |
| PERF-001 misleading tests | 2, 4–8 | Integrated P0–P8 results at each hot-stage gate |
| FAULT-001 aborting release panic | 3, 7–8 | Release panic injection plus supervised process-restart recovery |
| PERSIST-001 browser export buffering | 2, 6–7 | A1, A2, A5, A10 and static path check |
| PERSIST-002 browser import buffering/limit | 2, 6–7 | A1, A3, A4, A7, A10 |
| PERSIST-003 unbounded database/storage | 2, 6–7 | A9 and owner-selected configurable retention |
| PERSIST-004 decimal population JSON | 3, 6–7 | A5, A6, A8 |
| PERSIST-005 false bounded-export claim | 2, 6–7 | A1, A2, A4, A10 |
| PERSIST-006 incomplete/non-atomic transfer | 3, 6–7 | A6 and A7 |
| PERSIST-007 repeated history/HoF genomes | 2, 6–7 | A5, A6, A9 |

## Error-handling policy

### Invalid initialization or reset

- Validate the complete graph, settings, counts, arithmetic, memory estimate,
  checkpoint versions, RNG state, and genome rows before constructing a new
  current world.
- Return one plain reason naming the rejected field/limit.
- Keep the previous valid world current.
- Do not partially replace settings, graph, or population.

### Engine calculation fault

- This section applies to a returned error or a Rust panic caught at the
  Stage 3 unwind boundary while the Node process remains alive.
- Mark the authoritative engine faulted at the last committed tick.
- Stop starting new fixed steps.
- Do not publish a success frame/stat/checkpoint after the failing partial
  step.
- Keep `/health` and a clear client error available.
- Do not switch to TypeScript or a different math path mid-generation.
- Recovery is explicit Reset, New Run, compatible restart, or owner-directed
  action.

### Rust worker failure

- Catch a worker panic at its thread root and treat a missing/panicked worker
  result as a failed step.
- Cancel/discard all uncommitted sibling results.
- Include worker/job identity in diagnostics without leaking unsafe internal
  data.
- Never commit a partial population result.

### Process-level native failure

- A native memory fault, explicit abort, double panic, or process-level OOM can
  terminate the in-process Node server; `/health` cannot honestly survive that.
- The Debian service supervisor restarts the process with bounded backoff.
- Startup validates and resumes only the latest fully committed checkpoint; it
  never treats a staged/temp/current-pointer mismatch as success.
- Browser and trainer connections observe disconnect/reconnect behavior, not a
  fabricated live engine-fault response.

### Persistence failure

- Keep the previous durable/current state and all public identity/events
  unchanged; retain or explicitly discard the isolated staged transition
  without consuming more RNG.
- Do not acknowledge Reset/New Run when its required run-start checkpoint
  failed.
- Report database failure separately from engine calculation health.
- Do not delete or automatically rewrite the user database.

### Slow or disconnected client

- Preserve controller lease according to monotonic elapsed-time state.
- Keep lifecycle, player, and compatible Protocol 2 traffic ahead of display
  frames.
- Retain only the newest unsent display frame and latest replaceable stats.
- If reliable delivery is impossible, close the socket with an explicit reason
  while preserving the reclaim lease.
- A spectator can be dropped without affecting the game.

### Overload

- Continue complete fixed steps while within safe memory.
- Bound scheduling debt and show achieved speed.
- Do not run interactive catch-up bursts merely to reduce the displayed debt.
- Keep accepting commands through Node.
- Never disable collision entries or hazard sensors.
- Never present “1x” as achieved when measurements show otherwise.

## Risk register

### RISK-001: Port preserves an existing hidden bug

**Mitigation:** Stage fixtures, known-defect list, real production integration
tests, and owner-visible gameplay rules before parity.

### RISK-002: Port invents simplified behavior again

**Mitigation:** Source-to-Rust map, reviewed source lines per module, no deletion
before fixtures, and full-stage comparisons.

### RISK-003: Two authoritative worlds drift

**Mitigation:** TypeScript reference is offline/test-only during comparison.
Production mode selects exactly one runtime; no per-tick synchronization
between two live worlds.

### RISK-004: Long synchronous N-API call recreates network starvation

**Mitigation:** Background Rust coordinator, nonblocking command queue, batched
event drain, and explicit Node responsiveness tests.

### RISK-005: Parallelism changes evolution/collision

**Mitigation:** Per-brain/per-snake owned writes, immutable detection snapshots,
deterministic commit, serial RNG/evolution initially, and worker-count
invariance tests.

### RISK-006: Rust is fast but browser/LAN remains unusable

**Mitigation:** v1 latest-frame/lifecycle priority plus browser
parse/render/network gates, not server-only timing. Promote culling/LOD before
cutover only if the measured v1 maximum fails those gates.

### RISK-007: Extreme settings exhaust 16 GB

**Mitigation:** Checked memory estimate, explicit supported envelope, hard
headroom, and reject-before-current behavior.

### RISK-008: Existing databases become unreadable

**Mitigation:** Keep current v2 rows, bounded legacy gzip/current-JSON readers,
fixture copies, non-destructive archive versioning, and A7/A8 round trips.

### RISK-009: External RL project breaks

**Mitigation:** Preserve Protocol 2 JSON flow; optional token fields are
backward compatible; coordinate any optional binary extension separately.

### RISK-010: LAN is mistaken for public hosting again

**Mitigation:** Direct LAN acceptance test from another device, explicit
`0.0.0.0` docs, and no unrelated public-security work in scope.

### RISK-011: Development spends months polishing temporary TypeScript

**Mitigation:** Stage 1 TypeScript work is limited to valid-oracle and urgent
control correctness. Performance architecture work belongs in Rust.

### RISK-012: The plan becomes authoritative-looking fiction

**Mitigation:** Draft/approval label, all work initially unchecked, evidence
required for status changes, no invented owner decisions, and measured results
kept separate from targets.

### RISK-013: SIMD/unsafe memory causes corruption

**Mitigation:** Safe scalar reference first, checked public boundaries, narrow
unsafe scopes with exact range/non-alias proof, fuzz/property tests, sanitizers
where practical, and release Clippy/rustfmt/test gates.

### RISK-014: Float differences defeat exact replay claims

**Mitigation:** Version exact replay to build/target/settings, keep discrete
ordering exact, use explicit numeric tolerance, and do not promise cross-CPU
bit identity.

### RISK-015: Scope expands into unrelated infrastructure

**Mitigation:** Non-goals exclude public auth/TLS/cloud. Production static file
serving and systemd guidance are included only because they directly support
the stated Debian/LAN deployment.

### RISK-016: Compression is claimed without measuring real genomes

**Mitigation:** P8 records ratios and encode/decode time for fresh and evolved
P0/P2/P3 data. Each payload falls back to raw packed when compression is not
smaller; no arbitrary raw-compression percentage is claimed. The archive does
not pass merely because a codec microbenchmark or highly compressible
synthetic buffer looks good.

### RISK-017: Retention deletes valued data

**Mitigation:** Implement the owner-selected configurable counts, interval,
byte cap, prior-run anchors, and separate Pin/Export meanings only after Stage
2 measurement and copied-data migration tests. Pinned checkpoints and
browser-downloaded exports are excluded by construction; A9 verifies every
retained root before and after pruning.

### RISK-018: Valid archive import partially changes the live run

**Mitigation:** Spool, validate, stage, durably prepare, and boundary-commit as
one correlated operation. Browser state changes only after success. A7 compares
the complete pre/post failure digest and database current pointer.

### RISK-019: A database-only backup omits managed checkpoints

**Mitigation:** Health/docs identify SQLite plus the managed directory as one
durable set. Cold backup copies both while stopped; hot backup holds an
inventory reference and validates every immutable root. Export remains the
portable one-experiment backup. Track 7D restores real copied backup sets.

## Approved technical recommendations

The owner decisions in the next section are settled inputs carried into Draft
4. The owner approved the technical implementation choices below as part of
revision `2026-07-29-draft-4`. Material changes still require review under the
plain change-review rule.

1. **One in-process Rust engine on background native threads.** This avoids a
   second service and extra per-step process messaging while keeping Node
   responsive.
2. **Node keeps WebSocket/HTTP and a dedicated persistence worker owns
   SQLite metadata.** These are the thin interface, not the authoritative game.
   Rust creates managed adaptive checkpoint files; Node routes files/results
   and owns small transactions without blocking its main event loop.
3. **Rust owns controller lease state.** Ownership affects every fixed step and
   must stay next to the authoritative world.
4. **Scalar correctness before concurrent execution.** Stages 3–6 establish a
   complete scalar path; Stage 7 compares one/four/five/six and useful
   seven-worker cases on the target VM.
5. **Physics begins in `f64`; neural data remains `f32`.** Optimize precision
   only with evidence.
6. **Exact serializer v1 first.** A negotiated exact-ID/culled/LOD format is
   post-cutover unless v1 fails the measured control/browser gate.
7. **TypeScript remains a selected porting/test oracle through migration and
   stabilization, never a production fallback after cutover.**
8. **New saves use one simple USTAR archive with adaptive raw-packed or
   shuffled-Zstandard numeric entries.** Population and Hall-of-Fame weights
   remain bit-exact binary; Node never needs codec support.
9. **Graph layout becomes explicitly versioned and locale-independent.**
   Legacy weights are mapped by proven node identity, never assumed string
   order.
10. **New checkpoint-v3 payloads are immutable managed files; SQLite stores
    metadata/current pointers/history/indexes/references.** Stage 2 may run
    only the narrow disposable SQLite byte-volume experiment defined above;
    it does not build a second checkpoint system. Managed files remain selected
    unless a concrete correctness/deployment problem or measured failure is
    reported and reviewed.
11. **Complete compact generation summaries are preserved.** High-volume
    diagnostics are not silently folded into exact state.
12. **Export prepares one bounded temporary archive behind the original normal
    download request. Import spools the original uploaded file and commits only
    after complete validation.**

## Owner decisions recorded for Draft 4

The owner selected the following rules in the 2026-07-28 message requesting
Draft 3. Draft 4 carries all 22 forward unchanged, and the owner's 2026-07-29
approval authorizes their implementation as part of this plan.

1. Hold the last accepted player/RL input for 500 ms wall time, then use
   neutral steering and boost-off. Reserve exclusive ownership for 30 wall-
   clock seconds after disconnect. Both values are configurable. No brain runs
   during grace.
2. After grace, perform one explicit neural takeover. The expired external
   controller cannot apply more input and external/brain outputs never mix.
3. Simultaneous head-to-head collision kills both snakes.
4. A contested pellet goes to the nearest eligible head; an exact tie uses
   stable snake ID.
5. The working kill-credit rule is body owner for head-to-body death and no
   kill for either participant in simultaneous head-to-head death. Stage 2 must
   show current behavior and examples before this provisional detail is
   finalized; any contrary intended rule returns for review.
6. Initial automatic retention keeps eight recent checkpoints, twelve
   milestone slots at 25-generation intervals, and two prior-run anchors,
   under a configurable 4 GiB cap. Pinned checkpoints and downloaded exports
   are never automatically deleted. Stage 2 checks the configurable defaults.
7. “Pin checkpoint” and “Export archive” are two distinct user-visible
   operations; neither silently performs the other.
8. Initial Hall-of-Fame retention is the best 50 unique genomes plus pinned
   entries. Preserve inexpensive compact metadata for older entries and prune
   no existing Hall-of-Fame data before verified migration.
9. Exact import restores the saved experiment identity when non-conflicting.
   Importing an older exact boundary where later generations exist creates an
   explicitly labelled new-run branch with provenance and never deletes the
   later suffix.
10. Hall of Fame is run-scoped. Exact restoration restores that run's Hall of
    Fame and never merges it into an unrelated global collection.
11. Preserve every compact eight-field generation summary without
    downsampling. Future high-frequency diagnostic history needs a separate
    reviewed retention rule.
12. P0, P1 and P2 are mandatory real-time targets on the Debian VM. P3 is an
    initial measured capacity case.
13. Keep TypeScript as a selectable reference/test oracle through migration
    and stabilization, remove it from production after cutover, and do not
    require destructive deletion later. It may be archived or removed after
    Rust remains stable.
14. Exact saves occur at generation boundaries. True mid-round save is outside
    current scope.
15. Frame v2 or advanced RL protocols enter the critical path only if measured
    production results prove the existing protocol prevents required latency
    or capacity.
16. Ordinary Export creates one self-contained resumable experiment. No
    population-only export is added without a demonstrated use.
17. Inventory the owner's real databases and save files before retiring or
    narrowing any compatibility reader.
18. Write one automatic resumable checkpoint every generation initially;
    retention prevents unbounded accumulation.
19. Reject an impossible collision-safe spawn request with a clear error and
    never silently reduce configured population.
20. Archive, decoded-state, temporary-disk, RSS, record and watchdog limits
    remain provisional/configurable through Stage 2. Final defaults derive
    from decoded state, record counts, actual free disk and the 16 GiB VM.
21. After Reset, New Run or import, keep sockets open, invalidate old
    assignments/epochs, send one reliable state-replaced result and put clients
    in `awaitingRejoin`.
22. Normal latest startup encountering a corrupt current checkpoint creates a
    clearly reported recovery branch from the newest valid retained
    checkpoint, preserves the corrupt record and abandoned future suffix, and
    reports recovered/lost generations. Explicit resume of a particular
    invalid ID fails without substitution.

### Plain change-review rule

For a correction that changes gameplay, selection pressure, controller
experience, retention/deletion, saved meaning, or the external RL contract,
record the current behavior, reproducing evidence, plain alternatives, and the
actual owner answer before the dependent behavior is finalized. After draft
approval, work may rely on the 22 recorded decisions; it may not silently
replace them. Evidence required by decision 5 or any later material change is
reported for review rather than treated as permission to choose a new rule.

## Revision history and factual implementation log

| Revision | Date | State | Summary |
|---|---|---|---|
| `2026-07-23-draft-1` | 2026-07-23 | Superseded; do not implement | Correct broad Rust direction, but wrong browser JSON persistence, unbounded database policy, late cutover, and excessive governance. |
| `2026-07-28-draft-2` | 2026-07-28 | Superseded; do not implement | Fixed direct transfer, browser memory, binary archives, retention/history, early Rust path and false owner history; still assumed SQLite checkpoint chunks and over-specified compression/integrity/Stage 3. |
| `2026-07-28-draft-3` | 2026-07-28 | Superseded; do not implement | Selected managed checkpoint files after an explicit SQLite comparison, adaptive raw/compressed entries, minimal integrity layers, narrower Stage 3, and recorded all 22 owner decisions. |
| `2026-07-29-draft-4` | 2026-07-29 | Approved for implementation; exact reviewed plan in `7971ed2ddbda86891c77def31d980aedf96b4236` | Adds independent browser-player transmission and welcome-allocation defects, removes the dual-persistence implementation detour, separates current-source/Git-history/prior-measurement evidence, and makes write-time archive validation explicitly measurement-gated while preserving Draft 3 architecture and all 22 owner decisions. |

The separate factual log at
`docs/todo/rust-authoritative-runtime-implementation-log.md` names the approved
revision and Git commit. Each stage needs only a compact row or short entry
containing date/commit, evidence command or artifact, measured result, known
issue/deviation, and any owner decision actually received. Do not use
“complete,” “accepted,” “verified,” or “owner-approved” without the evidence
or message that makes the word true.

## Required command families after implementation begins

Exact scripts may evolve, but every implementation stage must keep these
families green:

```powershell
cargo fmt --manifest-path native\Cargo.toml -- --check
cargo clippy --manifest-path native\Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path native\Cargo.toml --release
node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false
node .\node_modules\eslint\bin\eslint.js .
node .\node_modules\vite\bin\vite.js build
node .\node_modules\tsx\dist\cli.mjs scripts\run-tests.ts all --reporter=dot
```

Add explicit commands for:

- Rust engine integration tests;
- differential fixture tests;
- Protocol/API black-box tests;
- LAN tests;
- P0–P8 benchmarks;
- browser direct-download/upload heap acceptance;
- archive corruption, decompression-bomb, legacy migration, retention, and
  overnight database fixtures;
- Debian release build identity;
- memory/soak runs.

Passing commands are evidence only for the behavior they exercise.

## Definition of done

The project is not done merely because Rust code exists, tests are numerous,
or an isolated benchmark is fast. It is done under this plan only when all of
the following are true:

- The factual log records the owner's explicit go-ahead for the named revision
  and the Git commit that contains that revision.
- Active documentation no longer fabricates kernel-only owner approval.
- Normal production startup creates one Rust-owned game.
- Node has no production simulation, sensing, neural graph, physics,
  collision, or evolution loop.
- The Rust/Node boundary is coarse and nonblocking.
- All supported graph types run complete heterogeneous populations in Rust.
- Corrected body sensors reach evolved brains, baseline bots, players, and RL.
- Controller disconnect/reclaim/grace behavior passes under induced lag.
- Browser-player action sending is independent of sensor and display delivery;
  newest steering and boost release reach the server and next eligible
  fixed-step drain under deliberate sensor suppression.
- Critical assignments cannot be silently dropped behind frames.
- Collision has no silent capacity truncation or array-order outcome bias.
- Generation spawns are collision-safe for feasible settings.
- External connection bookkeeping does not advance world/evolution RNG.
- Fixed-step timing remains correct and P0/P1/P2 meet the agreed VM real-time
  gates.
- A configured 60-second supported round no longer takes several real minutes.
- Browser frame/network/render load meets the agreed laptop gate.
- Welcome-state refresh uses latest packed-frame length metadata and never
  serializes or reconstructs the complete world merely to calculate it.
- Memory remains safely inside the 16 GB VM allocation.
- Export is one direct packed adaptive file download and import uploads that
  original file; browser JavaScript never materializes population data.
- A1–A10 pass for default, large-brain, corruption, legacy, and
  overnight-equivalent fixtures.
- New checkpoints and Hall-of-Fame genomes use packed raw-or-compressed binary
  managed storage rather than decimal population JSON or SQLite chunk BLOBs.
- The owner-selected recent/milestone/prior-run/pinned policy bounds automatic
  managed storage, every generation receives its initial resumable checkpoint,
  and pruning never touches pinned data or downloaded files.
- Complete compact generation history survives pruning and archive round trip
  without unapproved downsampling.
- Managed-file reclamation/orphan cleanup, small SQLite/WAL behavior, backup
  sets, and explicit legacy compaction are measured and documented on an
  overnight-sized fixture.
- Existing Protocol 2, HTTP, settings, graph, persistence, reset, New Run,
  import/export, Hall of Fame, resurrection, God Mode, and visualization
  workflows pass.
- Browser-visible IDs remain exact and unique for every live snake.
- Laptop and desktop browsers connect over the trusted LAN.
- The separate desktop RL trainer completes its real API workflow.
- Current/compatible legacy databases are handled non-destructively.
- Debian build/start/stop/database-plus-managed-files backup instructions work
  from a clean checkout.
- Obsolete Node worker/per-layer/native-boundary code is not in production.
- TypeScript remains selectable as a reference/test oracle through migration
  and stabilization but is not reachable from normal production startup.
- Final docs state measured limits and remaining issues honestly.
