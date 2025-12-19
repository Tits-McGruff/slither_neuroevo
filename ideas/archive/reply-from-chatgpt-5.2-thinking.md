# Purpose

You are the Codex agent. The human developer is the person building the project. In this document, “you” refers to you, the Codex agent. References to “the human developer” refer to the person who will run, tweak, and experiment with the project.

This is the single canonical handoff document. It is the only context the human developer intends to paste into Codex. If any other handoff notes exist from earlier in this conversation, treat them as discarded.

You will do three things in this document. First, you will treat the original analysis report as an input and respond to it using what the code actually does today. Second, you will commit to one architecture direction to maximise compartmentalisation and minimise unrelated breakage. Third, you will produce an implementation sequence that avoids double handling, especially avoiding “build it once in untyped JS, then rebuild it again only to satisfy types.”

## Hard architectural decisions

The human developer’s stated goal is compartmentalisation so that changes do not break unrelated areas. The human developer also wants LAN multiplayer for humans, and wants bots and champions to connect as clients through the same interface.

You will implement a server-client architecture. The authoritative simulation runs as a Node server process. Browser pages are clients. Human LAN players are clients. Bot and champion programs are clients. This is a firm decision; you will not design a parallel “browser worker is the authoritative host” path.

You will implement authoritative persistence on the server. The server does not have access to browser localStorage or browser IndexedDB. Those exist only inside each browser client. The human developer wants a database because browser localStorage is small and because browser storage does not exist in the server process. You will implement SQLite in the server, and you will store run metadata and artifacts there. You will also support versioned export files on disk.

You will implement type checking at contract boundaries immediately. You will write a single protocol and contract module in TypeScript at the start. The rest of the codebase can remain JavaScript for hot loops and for gradual conversion, but all boundary work, protocols, schemas, and serialization constants will be driven by the typed protocol module from day one.

You will not introduce heavy platform and performance technologies in this plan. You will not implement WebGPU rendering, WebGPU compute, SharedArrayBuffer ring buffers, OffscreenCanvas rendering, or WebAssembly kernels in this plan. The human developer’s pain is unrelated breakage; these technologies expand surface area and regression risk and are not required for LAN clients or for external champions.

## What the code actually looks like right now

The current code is split between a main thread that handles UI and rendering and a simulation worker that owns the World and advances the simulation with a fixed-step accumulator. The worker emits a binary Float32Array snapshot every loop, and the main thread renders from that buffer using the fast-path renderer.

The binary frame contract is strict and currently includes view state. The serializer header is six floats in order: generation, total snake count, alive snake count, cameraX, cameraY, zoom. For each alive snake, the serializer writes eight floats: id, radius, skinFlag, x, y, dir, boostFlag, pointCount, followed by pointCount pairs of x,y for the polyline. Pellets begin with a pelletCount float, followed by pelletCount blocks of five floats: x, y, value, type, colorId. The renderer parses this with pointer arithmetic, including a two-pass approach where it skips the points to collect metadata, then revisits the points for drawing.

The worker message protocol already exists as a real but informal contract. It includes init, updateSettings, resize, viz enable, resurrect, and godMode. The main thread posts these message types, and the worker switches on msg.type.

Persistence is currently browser localStorage-first. Hall of Fame persists a capped list in localStorage. Population persistence exists in utilities, but the worker does not currently export or import the population live. The UI import path writes localStorage and reloads, and export is stubbed.

There is a legacy object-based render path that exists but is currently broken due to a stale helper reference. This is a concrete example of a maintenance hazard that causes unrelated breakage, because a code path that looks plausible is not actually correct.

The code uses Math.random in simulation-relevant places, including random genome initialization and incidental stochastic behavior. The Hall of Fame seed field is not a true RNG seed.

## Where the analysis report is correct, confirmed by the code

The analysis report correctly identifies the fragile surfaces. The most fragile surface is the binary frame contract. The code confirms this because the renderer relies on pointer math over a Float32Array and because the contract contains variable-length blocks. Drift in offsets, counts, or block order will produce subtle corruption rather than clear failures.

The analysis report correctly calls out import and export as incomplete and high leverage. The code confirms this because the authoritative population is in the worker, so any import/export that lives only in the main thread is inherently a workaround.

The analysis report correctly highlights that the worker messaging boundary is a regression risk. The code confirms this because the protocol is implemented as string literals and implicit payload shapes. As the message surface grows, accidental drift becomes more likely.

The analysis report’s testing emphasis is correct. You already have tests around serialization and parsing. Given the fragility of the buffer contract, tests that are directly targeted at pointer invariants and schema stability are unusually high value in this codebase.

## Response to the original analysis report, item by item

You will treat the report as a useful prioritisation guide, but you will reinterpret it through the server-client decision and through the actual code constraints.

On “finish incomplete features,” you will not interpret that as “complete every UI idea.” You will interpret it as “remove ambiguity.” Any half-implemented path should be either implemented, deleted, or hidden behind an explicit flag with tests. The highest value completion is import/export because it becomes the mechanism for external bots, champion checkpoints, and reproducible experiments.

On “moving tests into their own folder,” you will not do that in this plan. The only reason to do it is organisational preference, and it creates churn in relative imports. You will spend that time on contract tests instead, because that directly addresses the human developer’s main pain.

On “localStorage versus a database,” you will implement a server-side database because localStorage is small and because the server does not have browser storage. You will not implement browser IndexedDB as the authoritative persistence mechanism. SQLite on the server is the correct fit.

On “spawn a user snake,” you will implement it through the controller API, not by adding a special case inside evolution logic. In this architecture, “user snake” means “a client-controlled agent” with a controller id.

On “champion learner,” you will not embed a large online training loop inside the server. You will implement champions as external client programs that drive snakes via the controller API. This gives failure isolation and allows multiple champions without touching evolution.

On “pause mutations,” you will implement it as a server-side evolution scheduler switch, because evolution is server-side policy.

On “flexible brain architectures,” you will not build a general DAG architecture system in this plan. If the human developer wants to experiment, they can do it inside a champion client or by adding a small incremental architecture variation in the server’s evo-brain implementation later, but not as a large refactor now.

On “TypeScript partial adoption,” you will treat type checking as a boundary tool, not as a repository-wide conversion. The protocol module is TypeScript from the start. Server-client messages, artifact schemas, and frame schemas all flow through that module.

On “runtime schema validation,” you will validate on the boundaries only. Client actions and admin commands are validated and clamped. You will not validate inside per-tick per-entity loops.

On “Comlink or RPC abstraction,” you will not add Comlink. WebSockets and explicit message objects are the correct abstraction here because you want a real network boundary and you want messages to remain transparent.

On “SharedArrayBuffer,” “OffscreenCanvas,” and “WebGPU,” you will not implement any of them in this plan. They increase surface area. The only reason to revisit them is after the server-client refactor is stable and profiling proves a specific bottleneck.

On “WebAssembly kernels,” you will not implement them in this plan. Revisit only if server step time is proven to dominate and a small kernel is clearly isolated.

On “UI framework modernization,” you will not add a framework in this plan. The browser client’s primary job is rendering and sending inputs. You will keep UI changes small and local.

## What changes after looking at the code directly

The code changes the emphasis in a way that matters for implementation order.

The primary driver of “unrelated breakage” is contract drift, not missing features. The contract drift is currently expressed as duplicated knowledge. Buffer offsets and semantics exist in serializer comments and in renderer pointer math. Message schemas exist as implicit shapes in switch cases. Persistence schemas exist as whatever JSON happens to be written today. You will reduce unrelated breakage by centralising these contracts and enforcing them in code, not by adding more features first.

Determinism is an enabling technology, not a “nice to have.” The current code’s use of Math.random means that when a regression happens, reproduction is uncertain. That multiplies the perceived blast radius because the human developer cannot isolate whether a change caused the bug or whether randomness did. For a system that mixes physics, agents, and evolution, deterministic mode pays for itself by reducing debugging time.

The current frame contract includes camera state. That is acceptable for a single viewer. It is wrong for server-client with multiple viewers. This is not a minor concern. It forces a contract change early, because each client must own its own camera. You will remove cameraX, cameraY, and zoom from the authoritative frame schema and make client camera state purely client-side.

The legacy render path being half-broken is not merely untidy. It is a specific cause of unrelated breakage because any refactor that touches rendering can accidentally route execution through the legacy path or create confusion about what is “supported.” You will delete the broken legacy path.

## The chosen architecture and what it implies

You will run the authoritative simulation in Node and expose a WebSocket API.

The server will own:

World state, stepping, collision resolution, sensor computation, and evolution scheduling.

The server will also own persistence. It will store run metadata, Hall of Fame entries, population snapshots, and model checkpoints in SQLite. It will also export and import artifacts to and from disk.

The client will own:

Rendering, UI controls, camera state, and input device handling. Clients receive world snapshots and render them. Clients send control actions and admin requests.

Bots and champions will be clients. They receive observations and send actions. If a bot fails, only that bot fails. The server keeps stepping and other clients continue.

This architecture implies a clean separation between two data planes.

The render plane is a snapshot stream. It is designed for efficient rendering and can be lossy as long as it is visually coherent.

The control plane is an observation and action protocol. It is designed for correctness, versioning, and failure isolation.

You will not mix them. You will not make the bot protocol depend on the render snapshot layout, and you will not make the render protocol depend on bot training needs.

## Contracts you will make explicit and enforce

You will define three contracts as first-class and versioned.

The controller API schema. This is the interface for humans and bots. It is minimal and stable. It is versioned. It includes a tick identifier, an agent identifier, an observation payload, and an action payload.

The render frame schema. This is the interface for rendering clients. It is versioned. It is self-describing enough to parse safely. It does not include per-client camera state.

The persistence artifact schema. This is the interface for stored records and exported files. It is versioned. It includes metadata required to interpret saved brains and populations.

All three contracts live in one shared protocol module written in TypeScript. All code that reads and writes these contracts imports from that module.

You will also implement runtime validation for these contracts at the boundary. Type checking prevents developer mistakes at build time. Runtime validation prevents malformed clients and malformed imports from corrupting the server at runtime.

## Controller API design

You will implement a strict tick-based controller interface.

The server steps the simulation at a fixed tick rate. Each tick produces observations for controlled agents. The server accepts actions keyed by tick and agent id.

Late and missing actions are handled deterministically. The server never stalls waiting for clients. If an action is missing for a tick, the server reuses the last action or applies a default action. If a client is missing for longer than a timeout, the server despawns the client-controlled agent.

Observations are based on raw sensor vectors plus a small stable metadata header. The observation payload does not contain full internal world state. The human developer wants an API that is stable while the internal sim changes. Raw sensors provide that stability.

Debugging and inspection do not belong in the controller contract. You will provide a separate debug or admin channel for rich world state, tracing, and introspection. Controller clients must not depend on debug state.

You will version the controller schema separately from the render frame schema. A change to the renderer snapshot layout must not force a change to the controller API, and a controller API evolution must not force a renderer rewrite.

## Champion and bot strategy under this architecture

The human developer wants to experiment with many techniques and technologies, but wants failures isolated. The architecture choice implies a specific champion strategy.

Champions are external client programs. They connect to the server like humans. They receive observations and send actions. If a champion crashes or sends nonsense, the server clamps or ignores it and continues stepping. The champion’s snake either continues under default actions or is despawned.

Training is not performed inside the authoritative server loop. Training runs in a client-side program that uses the same controller API but can run headless and faster than real time by requesting a headless evaluation mode or by running many episodes with rendering disabled.

You will implement a staged export workflow for opponents and checkpoints.

The server can export static “opponent brains” that represent snapshots of evolved populations. A champion trainer can load a pool of opponent exports and train against them without needing to modify the server’s evolution code.

The champion trainer exports champion checkpoints. The server can load a champion checkpoint and spawn one or more champion-controlled agents that run those weights.

This staged approach avoids tight co-evolution coupling inside the server tick loop and keeps the evolution engine stable while still allowing an arms race through periodic opponent pool refresh.

## Render frame schema design

You will stream snapshots to rendering clients.

The authoritative frame schema will contain world coordinates for snakes and pellets, plus metadata such as tick and generation. It will not include cameraX, cameraY, or zoom.

Each client maintains its own camera. This allows multiple viewers to watch the same world with different viewpoints without forking the server.

You will still use a binary frame format for rendering clients. The current Float32Array approach is workable, but you will move its definition into the protocol module and you will include a schema version at the start of every frame.

You will include explicit counts and bounds checks. Parsing code must never read out of bounds even if a client receives a truncated or corrupted frame.

## Server persistence: SQLite schema and artifact handling

You will implement SQLite as the authoritative persistence mechanism.

You will store these categories of data:

Runs and metadata. This includes start time, seed, settings snapshot, protocol schema versions, and summary metrics.

Hall of Fame entries. This includes a reference to the run, the genome or brain parameters, fitness metrics, and any display metadata.

Population snapshots. This includes generation index, a compressed or binary representation of genomes, and required metadata to decode.

Champion checkpoints. This includes model parameters, architecture metadata, sensor and action schema versions, and evaluation scores.

You will treat persisted records as contracts. Every stored row and every exported file includes at least schemaVersion, sensorSchemaVersion, actionSchemaVersion, architectureId, and dimensions. On load, the server validates and either migrates or rejects.

You will implement export and import as server operations. The browser client can request an export and receive a file or payload, but the write happens on the server. This aligns with the server being authoritative.

## Type checking strategy that avoids double handling

You will not postpone type checking until after major work is done.

You will create the protocol module in TypeScript first. It defines message names, payload types, schema versions, and frame layout constants.

You will then refactor existing code to consume those types and constants. You will do this before implementing the server-client move, so that the server and client are both built on the same typed definitions.

You will keep simulation hot loops in JavaScript as needed. The goal is boundary safety, not full conversion.

You will add runtime validation at boundaries using a schema validator. The validator must run only on low-frequency messages and on client actions, not inside per-entity per-tick loops.

## Determinism and reproducibility

You will implement a deterministic seeded RNG in the sim core and route all simulation randomness through it. You will not use Math.random in sim-critical logic.

You will persist the seed in the run record. You will include the seed in exports. You will log the seed in a way that the human developer can paste it back in to reproduce a run.

You will also implement a minimal evaluation harness for comparing brains and checkpoints. This does not need to be fancy. It needs to make “apples to apples” comparisons possible. The server will support a headless evaluation mode where it runs a fixed number of episodes using a fixed set of evaluation seeds and outputs summary metrics. Champion clients and exported brains can be benchmarked with the same fixed seeds.

Without determinism and a small evaluation harness, changes can appear to break unrelated things because runs are not comparable. Determinism reduces debugging time and reduces false conclusions during experimentation.

## How to interpret profiling numbers

You will collect timing metrics for server tick time, serialization time, send time, client parse time, and client render time. You will interpret them as a decision rule, not as vanity metrics.

If client render dominates, you will not change server logic. You will focus on rendering simplifications, draw batching, and frame throttling.

If client parse dominates, you will focus on frame schema layout and parsing strategy, including bounds checks and avoiding unnecessary passes.

If serialization dominates on the server, you will focus on how snapshots are constructed and on reducing per-tick allocation.

If server tick time dominates, you will focus on algorithmic changes in the sim and only later consider native acceleration.

If send time dominates, you will focus on frame frequency, compression strategy, and payload sizes, not on sim math.

## Implementation sequence

The goal of the sequence is to minimise unrelated breakage and minimise rework. Each phase reduces uncertainty in the next phase.

### Phase A: centralise contracts and make them executable

You will create the TypeScript protocol module and move all message type names and payload shapes into it.

You will move all frame layout constants and parsing assumptions into it. The renderer will no longer have private knowledge of offsets that the serializer does not share.

You will add a schema version field to the frame header and to every protocol message.

You will implement runtime validation at boundaries, especially for client actions, import payloads, and settings updates.

You will add a deterministic RNG and remove Math.random from sim-critical code.

You will add dev-only timing instrumentation for server tick time, serialization time, network send time, client parse time, and client render time.

You will delete the broken legacy render path.

### Phase B: extract sim core to be runnable in Node

You will refactor the simulation core so it can run in Node. World stepping, collision, sensing, and evolution scheduling must not depend on browser-only APIs.

You will create a Node server wrapper that owns the simulation loop and exposes WebSocket endpoints.

You will create a browser client wrapper that connects to the server, receives snapshots, and renders.

You will remove camera fields from the authoritative snapshot schema and implement camera purely client-side.

### Phase C: implement controller API and external clients

You will implement the controller API for client-controlled snakes. The server will provide observations and accept actions.

You will implement late and missing action behavior in the server. The server will never stall.

You will implement a human controller client that sends actions based on keyboard or gamepad.

You will implement a bot controller client scaffold that connects, receives sensor observations, and sends actions.

You will implement “pause mutations” as a server-side evolution scheduler switch.

### Phase D: server persistence and artifact flows

You will implement SQLite and schema migrations.

You will implement server endpoints for export and import of populations and checkpoints.

You will implement versioned artifact encoding and decoding. The server will reject artifacts that do not match supported schema versions.

### Phase E: regression protection

You will implement property-based tests for the render frame schema and its parsing invariants.

You will implement integration tests that start the server, connect a client, and verify frame cadence and settings propagation.

You will implement Playwright tests for the browser client connecting to the server and rendering.

## Things you will not do

You will not build persistence around browser localStorage or browser IndexedDB.

You will not maintain multiple overlapping controller protocols.

You will not keep half-working legacy paths.

You will not embed a complex online champion training loop inside the server. Champion training logic lives in external clients. The server remains an environment plus evolution scheduler.

You will not implement WebGPU, SharedArrayBuffer ring buffers, OffscreenCanvas rendering, or WebAssembly kernels in this plan.

## Deferred technology notes that were discussed earlier

These notes exist to capture prior discussion so the Codex agent does not re-open the same debates mid-implementation.

WebGPU is not CUDA. It does not provide CUDA libraries or direct access to vendor tensor libraries. It does provide programmable graphics and compute shaders. That makes it viable for custom numeric kernels, but it comes with high complexity in buffer layout, synchronisation, and debugging. It is deliberately excluded from this plan.

SharedArrayBuffer requires cross-origin isolation headers and introduces deployment constraints. It is deliberately excluded from this plan.

OffscreenCanvas can move rendering off the main thread, but it increases coordination complexity. It is deliberately excluded from this plan.

WebAssembly kernels can accelerate small numeric hot spots, but they add a build toolchain and debugging friction. They are deliberately excluded from this plan.

If the human developer later chooses to explore any of these technologies, it should be done after the server-client architecture is stable and after profiling data indicates a specific bottleneck. That later exploration should be treated as a separate branch or separate project.

## Terminology note for the report title

If the human developer wants a three-word framing for the analysis report title, the missing word is suitability. The intended phrase is “feasibility, suitability, and triage analysis.”

## Closing note

The current code already has the right overall instinct, it separates simulation from rendering. Your job is to make the boundaries explicit, versioned, and test-protected, then move the authoritative sim into a Node server so humans and bots can connect through a narrow API. This is the most direct path to compartmentalisation and to reducing unrelated breakage while the human developer experiments with neural network techniques.
