# Phase 1 plan: Server skeleton (authoritative sim + WebSocket)

## Purpose and scope

This phase delivers a standalone Node server that owns the simulation loop and
streams serialized frames to clients over WebSocket. The server is the single
source of truth for world state. The browser becomes a renderer that simply
consumes frames and displays them, and there is no external control input yet.

The goal is not to add gameplay features, persistence, or player identity. The
goal is to stand up the smallest correct, testable, and debuggable server that
can run the World loop continuously and provide a frame stream for the UI.

## Constraints and assumptions

The server must run as a single Node process with no access to DOM APIs. The
simulation should run at a fixed timestep for determinism, and the rendering
rate should be decoupled from the simulation rate so the UI can run at a
smoother, lower frame rate without affecting the sim. In this phase there is
no database, no auth, no multi-session support, and no remote control logic.

## Architecture narrative

At runtime the server spins up an HTTP server and a WebSocket server. The HTTP
server only exists to answer `/health` so a client or test can detect that the
process is alive. The WebSocket server accepts connections, performs a strict
handshake, and then broadcasts binary frames to UI clients. The simulation loop
is scheduled on a fixed interval using a drift-compensated timer. Each tick
runs `World.update(dt)`, serializes the world into a binary buffer, and sends
that buffer to UI clients at a rate-limited cadence. Stats are emitted once per
second as a small JSON message to all clients.

## Decisions locked for Phase 1

This phase uses a single world per server instance with open access and no API
keys. The tick rate is configurable with a default of 60 Hz, and UI frames are
rate-limited to 30 Hz by default. The WebSocket protocol uses JSON for control
and metadata and binary frames for rendering. HTTP is limited to `/health`.

## Data flow and timing

The flow below describes the hot path for each tick. It is intentionally
minimal and uses the existing serializer to avoid allocations.

```text
WS clients connect -> handshake -> server tick loop -> serialize -> broadcast
```

Tick loop timing uses a drift-compensated `setTimeout` rather than `setInterval`
so it can recover from transient delays. `performance.now()` is used to compute
accurate deadlines.

## Module map and responsibilities

### Pseudocode: `server/index.ts`

`index.ts` is the entry point and orchestrator. It wires the HTTP server, WS
hub, and simulation server together, then starts the loop and attaches shutdown
handlers. It should contain no business logic beyond bootstrapping.

Function sketch:

```ts
export async function main(): Promise<void>;
```

Behavior: parse config, create HTTP server, create WS hub, create sim server,
start tick loop, and handle SIGINT/SIGTERM for clean shutdown.

### Pseudocode: `server/config.ts`

This module defines the server config schema and parses CLI and environment
values. Parsing is deterministic and never throws; invalid values fall back to
defaults and are logged once during startup.

Types and functions:

```ts
export type LogLevel = "debug" | "info" | "warn" | "error";

export interface ServerConfig {
  port: number;
  tickRateHz: number;
  uiFrameRateHz: number;
  logLevel: LogLevel;
  seed?: number;
}

export function parseConfig(argv: string[], env: NodeJS.ProcessEnv): ServerConfig;
```

Validation rules: `port` must be in 1..65535, `tickRateHz` in 1..240,
`uiFrameRateHz` in 1..240, and `uiFrameRateHz` must be less than or equal to
`tickRateHz`.

### Pseudocode: `server/protocol.ts`

Defines JSON message types and manual validators. Validators are strict and
reject malformed shapes, missing fields, and non-finite numbers. The protocol
layer is intentionally small to minimize error handling complexity.

Client -> server messages:

- `hello`: identifies client type and protocol version.
- `join`: chooses spectator or player mode.
- `ping`: optional keepalive.

Server -> client messages:

- `welcome`: includes tick rate, seed, and sensor schema metadata.
- `stats`: heartbeat message.
- `error`: protocol error and disconnect reason.

Example `hello` message:

```json
{ "type": "hello", "clientType": "ui", "version": 1 }
```

Validation functions:

```ts
export function parseClientMessage(raw: unknown): ClientMessage | null;
export function isHello(msg: unknown): msg is HelloMsg;
export function isJoin(msg: unknown): msg is JoinMsg;
export function isPing(msg: unknown): msg is PingMsg;
```

### Pseudocode: `server/wsHub.ts`

The WS hub owns the WebSocket server and connection lifecycle. It enforces
protocol ordering, routes valid messages to the simulation server, and performs
broadcasts of frames and stats. The hub keeps lightweight connection state with
client type and handshake status.

Connection state shape:

```ts
export interface ConnectionState {
  id: number;
  socket: WebSocket;
  clientType: "unknown" | "ui" | "bot";
  joined: boolean;
  lastMessageTime: number;
}
```

Behavior: require `hello` before `join`, and close the socket on any protocol
violation. Only UI clients receive binary frames; all clients receive stats.

### Pseudocode: `server/simServer.ts`

The sim server owns a `World` instance and runs the tick loop. It never parses
WebSocket messages. Its job is to step the world, serialize, and publish.

Fields:

- `world: World`
- `tickRateHz: number`
- `uiFrameRateHz: number`
- `tickId: number`
- `lastFrameSentAt: number`
- `lastStatsSentAt: number`

Tick loop pseudocode:

```ts
const dt = 1 / config.tickRateHz;
let nextTick = performance.now();

function loop() {
  const now = performance.now();
  if (now >= nextTick) {
    world.update(dt);
    const frame = serialize(world);
    if (now - lastFrameSentAt >= 1000 / config.uiFrameRateHz) {
      wsHub.broadcastFrame(frame);
      lastFrameSentAt = now;
    }
    if (now - lastStatsSentAt >= 1000) {
      wsHub.broadcastStats(buildStats());
      lastStatsSentAt = now;
    }
    nextTick += 1000 / config.tickRateHz;
  }
  setTimeout(loop, Math.max(0, nextTick - now));
}
```

### Pseudocode: `server/controllerRegistry.ts`

This module exists as a placeholder for Phase 3. In Phase 1 it simply returns
`null` for control, meaning the default brain controls the snake. Keeping the
module now prevents disruptive refactors later.

Function shape:

```ts
export function resolveControl(snakeId: number, tickId: number): null;
```

## Protocol details

The server responds to a successful handshake with a `welcome` message that
includes the tick rate and a sensor schema placeholder. Even though sensors are
not used in Phase 1, the field exists to avoid protocol churn later.

Example `welcome` message:

```json
{
  "type": "welcome",
  "sessionId": "abc123",
  "tickRate": 60,
  "worldSeed": 12345,
  "cfgHash": "...",
  "sensorSpec": { "sensorCount": 128, "order": [] }
}
```

Binary frames are sent as raw `ArrayBuffer` payloads and must not be JSON
encoded. The client differentiates binary frames from JSON by the message type
of the WebSocket event.

## Logging and observability

Logging is structured as `timestamp | level | module | message` with a minimal
logger. Only important lifecycle events are logged at `info` and above. The
sim loop itself should not log per tick. A short debug mode can be added later
for troubleshooting.

## Detailed design notes

The handshake and message validation layer is intentionally strict because it
protects the hot loop from malformed input. Even though Phase 1 does not accept
actions, a strict handshake ensures future phases can assume a well-formed
connection state. Connections that skip `hello` or `join` are immediately closed
after a single `error` response to avoid slow-loris style connections or
inconsistent internal state.

Binary frame broadcasting must be treated as a best-effort operation. If a UI
client is slow or its socket buffer grows, the server should skip sending that
frame rather than blocking the loop. The WS hub should track `socket.bufferedAmount`
and drop frames when it exceeds a small threshold to avoid backpressure from
stalling the simulation loop.

The serializer must be imported and used as-is to preserve the existing buffer
contract. No changes to the binary layout should be made in Phase 1, and all
rendering code is expected to parse the same offsets. This is why the server
does not attempt any per-client transformations on the frame buffer.

## Error handling and shutdown

Server shutdown is explicit: on SIGINT or SIGTERM, the sim loop stops, sockets
close, and the HTTP server stops accepting connections. The shutdown path should
never throw; if something fails, it should log and continue closing remaining
resources. This makes dev iterations predictable and avoids port lockups.

## Testing plan

Unit tests validate protocol parsing, including acceptance of valid messages
and rejection of malformed structures or non-finite numbers. Integration tests
start the server, connect a WebSocket client, perform a handshake, and verify
that at least one binary frame arrives within one second.

Test files:

- `server/protocol.test.ts`
- `server/integration.test.ts`

## Footguns and safeguards

- Do not import DOM-only modules on the server.
- Never JSON stringify frame buffers.
- Avoid blocking the event loop inside the tick loop.
- Only UI clients should receive frames.

## Acceptance criteria

- `npm run server:dev` starts the server and logs startup.
- A WebSocket client receives a `welcome` message and frames.
- Unit and integration tests pass.

## Execution checklist

- [ ] Create `server/` directory and files
- [ ] Implement config parsing and validation
- [ ] Implement protocol validators
- [ ] Implement wsHub connection lifecycle
- [ ] Implement simServer tick loop
- [ ] Broadcast frames and stats
- [ ] Add server scripts to `package.json`
- [ ] Add protocol + integration tests

## Function-by-function pseudocode

### `server/index.ts`

The entry point is intentionally minimal and only coordinates other modules.
It never touches simulation internals directly.

```text
function main():
  config = parseConfig(argv, env)
  httpServer = createHttpServer(config.port)
  wsHub = new WsHub(httpServer, config)
  simServer = new SimServer(config, wsHub)
  simServer.start()
  on SIGINT/SIGTERM:
    simServer.stop()
    wsHub.closeAll()
    httpServer.close()
```

### `server/config.ts`

```text
function parseConfig(argv, env):
  port = parseInt(getArg("--port") or env.PORT or 5174)
  tickRate = parseInt(getArg("--tick") or env.TICK_RATE or 60)
  uiRate = parseInt(getArg("--ui-rate") or env.UI_RATE or 30)
  logLevel = getArg("--log") or env.LOG_LEVEL or "info"
  sanitize and clamp values
  return config
```

### `server/protocol.ts`

```text
function parseClientMessage(raw):
  if raw is not object: return null
  if raw.type is not string: return null
  switch raw.type:
    case "hello": return validateHello(raw)
    case "join": return validateJoin(raw)
    case "ping": return validatePing(raw)
    default: return null

function validateHello(msg):
  if msg.version !== 1: return null
  if msg.clientType not in {"ui","bot"}: return null
  return msg
```

### `server/wsHub.ts`

```text
function onConnection(socket):
  state = { id, socket, clientType:"unknown", joined:false }
  socket.onMessage(msg): handleMessage(state, msg)
  socket.onClose(): cleanup(state)

function handleMessage(state, raw):
  msg = parseClientMessage(raw)
  if msg is null:
    send error and close
  else if msg.type == "hello" and state.clientType == "unknown":
    state.clientType = msg.clientType
    send welcome
  else if msg.type == "join" and state.clientType != "unknown":
    state.joined = true
  else if msg.type == "ping":
    respond with pong or ignore
  else:
    send error and close

function broadcastFrame(buffer):
  for each state where clientType == "ui" and joined:
    if socket.bufferedAmount is small:
      socket.send(buffer)
```

### `server/simServer.ts`

```text
function start():
  nextTick = now()
  loop()

function loop():
  if now() >= nextTick:
    tick()
    nextTick += 1000 / tickRateHz
  setTimeout(loop, max(0, nextTick - now()))

function tick():
  world.update(dt)
  frame = serialize(world)
  if shouldSendFrame(): wsHub.broadcastFrame(frame)
  if shouldSendStats(): wsHub.broadcastStats(stats)
```

### `server/controllerRegistry.ts`

```text
function resolveControl(snakeId, tickId):
  return null
```

## Error handling and edge cases

The server rejects any JSON message that does not conform to the protocol
schema or contains non-finite numbers. A connection that sends `join` before
`hello` is rejected immediately to avoid ambiguous state. If the WS send buffer
for a UI client is large, the server skips that frame rather than blocking the
loop. If `tickRateHz` or `uiFrameRateHz` are invalid, the server clamps them to
safe defaults and logs a warning once at startup.

## Sample payloads and example WS session transcript

Sample JSON payloads:

```json
{ "type": "hello", "clientType": "ui", "version": 1 }
```

```json
{ "type": "join", "mode": "spectator" }
```

```json
{ "type": "stats", "tick": 120, "gen": 3, "alive": 42, "fps": 60 }
```

Example WS session transcript:

```text
client -> server: {"type":"hello","clientType":"ui","version":1}
server -> client: {"type":"welcome","sessionId":"abc","tickRate":60,"worldSeed":123,"cfgHash":"...","sensorSpec":{"sensorCount":128,"order":[]}}
client -> server: {"type":"join","mode":"spectator"}
server -> client: <binary frame>
server -> client: {"type":"stats","tick":120,"gen":3,"alive":42,"fps":60}
```

## Test matrix

| Test name | Setup / input | Expected result | Failure cases to verify |
| --- | --- | --- | --- |
| protocol_hello_valid | Send a valid `hello` with clientType `ui` | `parseClientMessage` returns HelloMsg | Missing `version` or wrong `clientType` is rejected |
| protocol_hello_invalid_number | Send `hello` with `version` as NaN | Message rejected and error sent | NaN slips through and corrupts state |
| ws_handshake_order | Send `join` before `hello` | Connection closed with error | Connection left open in invalid state |
| sim_frame_broadcast | Start server and connect a UI client | Client receives binary frame within 1 second | Frame sent as JSON or not sent at all |
| stats_rate_limit | Wait 2 seconds | Exactly 1 stats message per second | Stats flood client or never arrive |
